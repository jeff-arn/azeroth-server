"""Azeroth Private Server Dashboard — Flask backend."""

import functools
import hashlib
import os
import re
import subprocess

import mysql.connector
from flask import Flask, jsonify, redirect, render_template, request, session, url_for

app = Flask(__name__)
app.secret_key = os.environ.get("FLASK_SECRET_KEY", os.urandom(32))

DB_CONFIG = {
    "host": os.environ.get("DB_HOST", "127.0.0.1"),
    "user": os.environ.get("DB_USER", "mangos"),
    "password": os.environ["DB_PASSWORD"],
    "port": int(os.environ.get("DB_PORT", "3306")),
}

LEVELUP_LOG = os.environ.get("LEVELUP_LOG", "/home/wow/wow-private-server/vmangos/run/bin/LevelUp.log")

RACES = {1: "Human", 2: "Orc", 3: "Dwarf", 4: "Night Elf", 5: "Undead", 6: "Tauren", 7: "Gnome", 8: "Troll"}
CLASSES = {
    1: "Warrior",
    2: "Paladin",
    3: "Hunter",
    4: "Rogue",
    5: "Priest",
    7: "Shaman",
    8: "Mage",
    9: "Warlock",
    11: "Druid",
}
CLASS_COLORS = {
    1: "#C69B6D",
    2: "#F48CBA",
    3: "#AAD372",
    4: "#FFF468",
    5: "#FFFFFF",
    7: "#0070DD",
    8: "#3FC7EB",
    9: "#8788EE",
    11: "#FF7C0A",
}
ALLIANCE_RACES = {1, 3, 4, 7}
QUALITY_NAMES = {0: "Poor", 1: "Common", 2: "Uncommon", 3: "Rare", 4: "Epic", 5: "Legendary"}
QUALITY_COLORS = {0: "#9d9d9d", 1: "#ffffff", 2: "#1eff00", 3: "#0070dd", 4: "#a335ee", 5: "#ff8000"}

ZONE_NAMES = {
    1519: "Stormwind",
    1537: "Ironforge",
    1657: "Darnassus",
    1637: "Orgrimmar",
    1638: "Thunder Bluff",
    1497: "Undercity",
    12: "Elwynn Forest",
    40: "Westfall",
    38: "Loch Modan",
    44: "Redridge Mountains",
    10: "Duskwood",
    11: "Wetlands",
    148: "Darkshore",
    141: "Teldrassil",
    267: "Hillsbrad Foothills",
    33: "Stranglethorn Vale",
    331: "Ashenvale",
    3: "Badlands",
    8: "Swamp of Sorrows",
    45: "Arathi Highlands",
    440: "Tanaris",
    357: "Feralas",
    361: "Felwood",
    28: "Western Plaguelands",
    139: "Eastern Plaguelands",
    46: "Burning Steppes",
    51: "Searing Gorge",
    618: "Winterspring",
    1377: "Silithus",
    490: "Un'Goro Crater",
    405: "Desolace",
    406: "Stonetalon Mountains",
    15: "Dustwallow Marsh",
    400: "Thousand Needles",
    36: "Alterac Mountains",
    130: "Silverpine Forest",
    85: "Tirisfal Glades",
    14: "Durotar",
    215: "Mulgore",
    17: "The Barrens",
    47: "The Hinterlands",
    1: "Dun Morogh",
    1581: "The Deadmines",
    718: "Wailing Caverns",
    209: "Shadowfang Keep",
    491: "Razorfen Kraul",
    717: "The Stockade",
    1583: "Blackfathom Deeps",
    721: "Gnomeregan",
    2100: "Maraudon",
    1584: "Blackrock Depths",
    1585: "Blackrock Spire",
    2257: "Dire Maul",
    2057: "Scholomance",
    2017: "Stratholme",
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
        return jsonify(
            {
                "online": online,
                "uptime_seconds": uptime_seconds,
                "uptime_display": format_uptime(uptime_seconds),
                "players_online": players,
                "bots_online": bots,
            }
        )
    except Exception as e:
        return jsonify(
            {"online": False, "uptime_display": "0h 0m", "players_online": 0, "bots_online": 0, "error": str(e)}
        )


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
            players.append(
                {
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
                }
            )
        return jsonify(players)
    except Exception:
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
        class_map = {
            "c1": "Warrior",
            "c2": "Paladin",
            "c3": "Hunter",
            "c4": "Rogue",
            "c5": "Priest",
            "c7": "Shaman",
            "c8": "Mage",
            "c9": "Warlock",
            "c11": "Druid",
        }
        class_color_map = {
            "c1": "#C69B6D",
            "c2": "#F48CBA",
            "c3": "#AAD372",
            "c4": "#FFF468",
            "c5": "#FFFFFF",
            "c7": "#0070DD",
            "c8": "#3FC7EB",
            "c9": "#8788EE",
            "c11": "#FF7C0A",
        }
        events = []
        for line in reversed(lines):
            if "<BOT>" in line:
                continue
            match = pattern.search(line)
            if match:
                events.append(
                    {
                        "timestamp": match.group(1),
                        "name": match.group(2),
                        "class": class_map.get(match.group(3), "Unknown"),
                        "class_color": class_color_map.get(match.group(3), "#ffffff"),
                        "level": int(match.group(5)),
                    }
                )
            if len(events) >= 10:
                break
        return jsonify(events)
    except Exception:
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
            by_quality.append(
                {
                    "name": QUALITY_NAMES.get(quality, "Unknown"),
                    "color": QUALITY_COLORS.get(quality, "#ffffff"),
                    "count": count,
                }
            )
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
            raids.append(
                {
                    "name": info["name"],
                    "phase": info["phase"],
                    "unlocked": map_id not in disabled_maps,
                }
            )
        return jsonify(raids)
    except Exception:
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
    except Exception:
        return jsonify({"total_online": 0, "by_zone": []})


# ============================================================================
# Authentication — SRP6 password verification against realmd.account
# ============================================================================

# WoW 1.12 SRP6 constants
SRP6_N = int("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7", 16)
SRP6_g = 7


def _verify_srp6_password(username, password, s_hex, v_hex):
    """Verify password against stored SRP6 salt and verifier.

    Matches the C++ flow in SRP6::CalculateVerifier + AccountMgr::CalculateShaPassHash:
    1. rI = hex(SHA1("USERNAME:PASSWORD"))
    2. BigNumber I from rI hex -> BN_bn2bin (big-endian) -> AsByteArray reverses to LE
    3. Copy into mDigest[20], then std::reverse(mDigest) back to big-endian
    4. s: SetHexStr (big-endian) -> AsByteArray (reversed to little-endian)
    5. x = SHA1(s_LE || mDigest_BE)  -- A is zero/empty, contributes nothing
    6. v = g^x mod N
    """
    username = username.upper()
    password = password.upper()

    # Step 1: CalculateShaPassHash -> SHA1("USERNAME:PASSWORD") -> hex
    identity_hash = hashlib.sha1(f"{username}:{password}".encode()).digest()
    ri_hex = identity_hash.hex().upper()

    # Step 2: BigNumber I from hex. BN_bn2bin = big-endian, AsByteArray reverses to LE
    ri_int = int(ri_hex, 16)
    num_bytes = (ri_int.bit_length() + 7) // 8
    ri_be = ri_int.to_bytes(max(num_bytes, 1), byteorder="big")
    ri_le = ri_be[::-1]

    # Step 3: memset mDigest[20] = 0, memcpy LE bytes in, then std::reverse
    m_digest = bytearray(20)
    copy_len = min(len(ri_le), 20)
    m_digest[:copy_len] = ri_le[:copy_len]
    m_digest.reverse()  # back to big-endian

    # Step 4: salt from hex (big-endian) -> AsByteArray reverses to little-endian
    s_int = int(s_hex, 16)
    s_be = s_int.to_bytes(32, byteorder="big")
    s_le = s_be[::-1]

    # Step 5: x = SHA1(s_LE || mDigest_BE) -- A is empty, feeds 0 bytes
    x_hash = hashlib.sha1(s_le + bytes(m_digest)).digest()
    x = int.from_bytes(x_hash, byteorder="little")  # SetBinary = little-endian

    # Step 6: v = g^x mod N
    computed_v = pow(SRP6_g, x, SRP6_N)

    # ProofVerifier compares AsHexStr() = big-endian hex
    stored_v = int(v_hex, 16)
    return computed_v == stored_v


def authenticate_user(username, password):
    """Check credentials against realmd.account. Returns account dict or None."""
    try:
        conn = mysql.connector.connect(**DB_CONFIG, database="realmd")
        cursor = conn.cursor(dictionary=True)
        cursor.execute(
            "SELECT id, username, gmlevel, v, s FROM account WHERE username = %s",
            (username.upper(),),
        )
        account = cursor.fetchone()
        cursor.close()
        conn.close()

        if not account:
            return None

        if not _verify_srp6_password(username, password, account["s"], account["v"]):
            return None

        return account
    except Exception:
        return None


def get_user_guild_info(account_id):
    """Get the logged-in user's guild membership and rank."""
    try:
        conn = mysql.connector.connect(**DB_CONFIG, database="characters")
        cursor = conn.cursor(dictionary=True)
        cursor.execute(
            """SELECT gm.guild_id, gm.rank, c.guid, c.name AS char_name
               FROM characters c
               JOIN guild_member gm ON c.guid = gm.guid
               WHERE c.account = %s
               LIMIT 1""",
            (account_id,),
        )
        result = cursor.fetchone()
        cursor.close()
        conn.close()
        return result
    except Exception:
        return None


def login_required(f):
    """Decorator: redirect to login if not authenticated."""

    @functools.wraps(f)
    def decorated(*args, **kwargs):
        if "account_id" not in session:
            return redirect(url_for("login", next=request.path))
        return f(*args, **kwargs)

    return decorated


def guild_write_required(f):
    """Decorator: require guild master (rank 0) or officer (rank 1) for write ops."""

    @functools.wraps(f)
    def decorated(*args, **kwargs):
        if "account_id" not in session:
            return jsonify({"error": "Not authenticated"}), 401
        guild_rank = session.get("guild_rank")
        if guild_rank is None or guild_rank > 1:
            return jsonify({"error": "Insufficient guild rank"}), 403
        return f(*args, **kwargs)

    return decorated


@app.route("/login", methods=["GET", "POST"])
def login():
    error = None
    if request.method == "POST":
        username = request.form.get("username", "")
        password = request.form.get("password", "")
        account = authenticate_user(username, password)
        if account:
            session["account_id"] = account["id"]
            session["username"] = account["username"]
            session["gmlevel"] = account["gmlevel"]

            # Load guild info
            guild_info = get_user_guild_info(account["id"])
            if guild_info:
                session["guild_id"] = guild_info["guild_id"]
                session["guild_rank"] = guild_info["rank"]
                session["char_name"] = guild_info["char_name"]
            else:
                session["guild_id"] = None
                session["guild_rank"] = None
                session["char_name"] = account["username"]

            next_url = request.args.get("next", url_for("guild_page"))
            return redirect(next_url)
        else:
            error = "Invalid username or password."

    return render_template("login.html", error=error)


@app.route("/logout")
def logout():
    session.clear()
    return redirect(url_for("index"))


SLOT_NAMES = {
    0: "Head",
    1: "Neck",
    2: "Shoulder",
    3: "Shirt",
    4: "Chest",
    5: "Waist",
    6: "Legs",
    7: "Feet",
    8: "Wrists",
    9: "Hands",
    10: "Finger 1",
    11: "Finger 2",
    12: "Trinket 1",
    13: "Trinket 2",
    14: "Back",
    15: "Main Hand",
    16: "Off Hand",
    17: "Ranged",
    18: "Tabard",
}


@app.route("/guild")
@login_required
def guild_page():
    can_edit = session.get("guild_rank") is not None and session.get("guild_rank") <= 1
    return render_template(
        "guild.html",
        char_name=session.get("char_name", ""),
        can_edit=can_edit,
    )


@app.route("/api/guild/roster")
def api_guild_roster():
    try:
        conn = get_db("characters")
        cursor = conn.cursor(dictionary=True)
        cursor.execute("""
            SELECT r.guid, c.name, c.race, c.class AS cls, c.level,
                   r.spec, r.role, r.gear_score, r.gear_tier,
                   r.raid_assigned, r.active,
                   g.name AS guild_name, gm.rank AS guild_rank
            FROM guild_bot_roster r
            JOIN characters c ON r.guid = c.guid
            LEFT JOIN guild_member gm ON c.guid = gm.guid
            LEFT JOIN guild g ON gm.guild_id = g.guild_id
            WHERE r.active = 1
            ORDER BY r.role, c.class, c.name
        """)
        rows = cursor.fetchall()
        cursor.close()
        conn.close()
        roster = []
        for r in rows:
            roster.append(
                {
                    "guid": r["guid"],
                    "name": r["name"],
                    "race": RACES.get(r["race"], "Unknown"),
                    "class": CLASSES.get(r["cls"], "Unknown"),
                    "class_id": r["cls"],
                    "class_color": CLASS_COLORS.get(r["cls"], "#ffffff"),
                    "level": r["level"],
                    "spec": r["spec"] or "",
                    "role": r["role"] or "dps",
                    "gear_score": round(r["gear_score"], 1) if r["gear_score"] else 0,
                    "gear_tier": r.get("gear_tier", "dungeon_geared"),
                    "raid_assigned": bool(r["raid_assigned"]),
                    "active": bool(r["active"]),
                    "guild_name": r["guild_name"] or "",
                    "guild_rank": r["guild_rank"],
                    "faction": "Alliance" if r["race"] in ALLIANCE_RACES else "Horde",
                }
            )
        return jsonify(roster)
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route("/api/guild/bot/<int:guid>")
def api_guild_bot_detail(guid):
    try:
        conn = get_db("characters")
        cursor = conn.cursor(dictionary=True)
        cursor.execute(
            """
            SELECT r.guid, c.name, c.race, c.class AS cls, c.level,
                   r.spec, r.role, r.gear_score, r.raid_assigned, r.active
            FROM guild_bot_roster r
            JOIN characters c ON r.guid = c.guid
            WHERE r.guid = %s
        """,
            (guid,),
        )
        row = cursor.fetchone()
        if not row:
            cursor.close()
            conn.close()
            return jsonify({"error": "Bot not found"}), 404

        bot = {
            "guid": row["guid"],
            "name": row["name"],
            "race": RACES.get(row["race"], "Unknown"),
            "class": CLASSES.get(row["cls"], "Unknown"),
            "class_id": row["cls"],
            "class_color": CLASS_COLORS.get(row["cls"], "#ffffff"),
            "level": row["level"],
            "spec": row["spec"] or "",
            "role": row["role"] or "dps",
            "gear_score": round(row["gear_score"], 1) if row["gear_score"] else 0,
            "raid_assigned": bool(row["raid_assigned"]),
        }

        # Fetch equipped items
        cursor.execute(
            """
            SELECT ci.slot, ci.item_id, it.name, it.Quality, it.ItemLevel
            FROM character_inventory ci
            JOIN mangos.item_template it ON ci.item_id = it.entry
            WHERE ci.guid = %s AND ci.bag = 0 AND ci.slot < 19
            ORDER BY ci.slot
        """,
            (guid,),
        )
        items = cursor.fetchall()
        cursor.close()
        conn.close()

        gear = []
        for item in items:
            gear.append(
                {
                    "slot": SLOT_NAMES.get(item["slot"], f"Slot {item['slot']}"),
                    "item_id": item["item_id"],
                    "name": item["name"],
                    "quality": QUALITY_NAMES.get(item["Quality"], "Unknown"),
                    "quality_color": QUALITY_COLORS.get(item["Quality"], "#ffffff"),
                    "item_level": item["ItemLevel"],
                }
            )
        bot["gear"] = gear
        return jsonify(bot)
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route("/api/guild/bot/<int:guid>/spec", methods=["POST"])
@guild_write_required
def api_guild_bot_update_spec(guid):
    try:
        data = request.get_json()
        if not data:
            return jsonify({"error": "No data provided"}), 400
        spec = data.get("spec")
        role = data.get("role")
        if role and role not in ("tank", "healer", "dps"):
            return jsonify({"error": "Invalid role"}), 400
        conn = get_db("characters")
        cursor = conn.cursor()
        updates = []
        params = []
        gear_tier = data.get("gear_tier")
        if spec is not None:
            updates.append("spec = %s")
            params.append(spec)
        if role is not None:
            updates.append("role = %s")
            params.append(role)
        if gear_tier is not None:
            if gear_tier not in ("fresh_60", "dungeon_geared", "pre_raid_bis"):
                return jsonify({"error": "Invalid gear_tier"}), 400
            updates.append("gear_tier = %s")
            params.append(gear_tier)
        if not updates:
            return jsonify({"error": "Nothing to update"}), 400
        params.append(guid)
        cursor.execute(
            f"UPDATE guild_bot_roster SET {', '.join(updates)} WHERE guid = %s",
            params,
        )
        conn.commit()
        affected = cursor.rowcount
        cursor.close()
        conn.close()
        if affected == 0:
            return jsonify({"error": "Bot not found"}), 404
        return jsonify({"ok": True})
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route("/api/guild/raid-plan", methods=["POST"])
@guild_write_required
def api_guild_raid_plan():
    try:
        data = request.get_json()
        if not data or "assignments" not in data:
            return jsonify({"error": "No assignments provided"}), 400
        assignments = data["assignments"]  # list of {guid, raid_assigned}
        conn = get_db("characters")
        cursor = conn.cursor()
        # Clear all raid assignments first, then set the selected ones
        cursor.execute("UPDATE guild_bot_roster SET raid_assigned = 0 WHERE active = 1")
        if assignments:
            guids = [a["guid"] for a in assignments if a.get("raid_assigned")]
            if guids:
                placeholders = ", ".join(["%s"] * len(guids))
                cursor.execute(
                    f"UPDATE guild_bot_roster SET raid_assigned = 1 WHERE guid IN ({placeholders})",
                    guids,
                )
        conn.commit()
        cursor.close()
        conn.close()
        return jsonify({"ok": True})
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route("/api/guild/gear-tier", methods=["POST"])
@guild_write_required
def api_guild_gear_tier():
    """Set gear tier globally for all guild bots, or per-bot if guid provided."""
    try:
        data = request.get_json()
        if not data or "gear_tier" not in data:
            return jsonify({"error": "No gear_tier provided"}), 400
        gear_tier = data["gear_tier"]
        if gear_tier not in ("fresh_60", "dungeon_geared", "pre_raid_bis"):
            return jsonify({"error": "Invalid gear_tier"}), 400
        conn = get_db("characters")
        cursor = conn.cursor()
        guid = data.get("guid")
        if guid:
            cursor.execute(
                "UPDATE guild_bot_roster SET gear_tier = %s WHERE guid = %s",
                (gear_tier, guid),
            )
        else:
            cursor.execute(
                "UPDATE guild_bot_roster SET gear_tier = %s WHERE active = 1",
                (gear_tier,),
            )
        conn.commit()
        affected = cursor.rowcount
        cursor.close()
        conn.close()
        return jsonify({"ok": True, "updated": affected})
    except Exception as e:
        return jsonify({"error": str(e)}), 500


if __name__ == "__main__":
    from waitress import serve

    print("Azeroth Dashboard starting on port 8080...")
    serve(app, host="0.0.0.0", port=8080)
