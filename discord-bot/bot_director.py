#!/usr/bin/env python3
"""
Bot Director — Dynamically manages bot placement based on player activity.

Runs every 2 minutes. Checks where real players are, classifies their zones,
and assigns bots to create a living world around them.

Rules:
  - Capital city: wide level range (10-60), biased toward max level, passive
  - Friendly zone: player_level ± 5, same faction, passive
  - Contested zone: both factions, player_level ± 5, enemy bots aggressive
    - 10% chance of a level 60 bot (friendly or enemy) for tension
  - No players online: no bots assigned
"""

import mysql.connector
import random
import time
import logging
from datetime import datetime

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s [BotDirector] %(message)s'
)

DB_CONFIG = {
    "host": "127.0.0.1",
    "user": "mangos",
    "password": "fo2IquNCzghd0qBRYetHUtQw",
    "port": 3306,
}

# Zone classifications
CAPITAL_CITIES = {
    1519: {"name": "Stormwind", "map": 0, "x": -8829.5, "y": 625.6, "z": 93.9, "faction": "alliance"},
    1537: {"name": "Ironforge", "map": 0, "x": -4928.5, "y": -946.6, "z": 501.6, "faction": "alliance"},
}

FRIENDLY_ZONES = {
    12:  {"name": "Elwynn Forest", "map": 0, "x": -9449.0, "y": 64.0, "z": 56.0, "level_range": (1, 10)},
    40:  {"name": "Westfall", "map": 0, "x": -10647.0, "y": 1112.0, "z": 35.0, "level_range": (10, 20)},
    38:  {"name": "Loch Modan", "map": 0, "x": -5405.0, "y": -2894.0, "z": 341.0, "level_range": (10, 20)},
    44:  {"name": "Redridge Mountains", "map": 0, "x": -9278.0, "y": -2250.0, "z": 68.0, "level_range": (15, 25)},
    148: {"name": "Darkshore", "map": 1, "x": 6463.0, "y": 683.0, "z": 9.0, "level_range": (10, 20)},
    10:  {"name": "Duskwood", "map": 0, "x": -10531.0, "y": -1154.0, "z": 28.0, "level_range": (20, 30)},
    11:  {"name": "Wetlands", "map": 0, "x": -3799.0, "y": -822.0, "z": 10.0, "level_range": (20, 30)},
}

CONTESTED_ZONES = {
    267: {"name": "Hillsbrad Foothills", "map": 0, "x": -40.7, "y": -957.0, "z": 56.3, "level_range": (20, 30)},
    33:  {"name": "Stranglethorn Vale", "map": 0, "x": -12412.8, "y": 171.2, "z": 3.4, "level_range": (30, 45)},
    331: {"name": "Ashenvale", "map": 1, "x": 949.9, "y": -1435.7, "z": 62.7, "level_range": (18, 30)},
    3:   {"name": "Arathi Highlands", "map": 0, "x": -1263.0, "y": -2515.0, "z": 22.0, "level_range": (30, 40)},
    4:   {"name": "Badlands", "map": 0, "x": -6113.0, "y": -3205.0, "z": 255.0, "level_range": (35, 45)},
    440: {"name": "Tanaris", "map": 1, "x": -7193.4, "y": -3793.5, "z": 9.8, "level_range": (40, 50)},
    357: {"name": "Feralas", "map": 1, "x": -4808.0, "y": 1590.0, "z": 80.0, "level_range": (42, 50)},
    361: {"name": "Felwood", "map": 1, "x": 5230.0, "y": -546.0, "z": 343.0, "level_range": (48, 55)},
    28:  {"name": "Western Plaguelands", "map": 0, "x": 1741.0, "y": -1720.0, "z": 59.0, "level_range": (51, 58)},
    139: {"name": "Eastern Plaguelands", "map": 0, "x": 2280.0, "y": -5310.0, "z": 82.0, "level_range": (53, 60)},
    46:  {"name": "Burning Steppes", "map": 0, "x": -7530.0, "y": -1052.0, "z": 229.0, "level_range": (50, 58)},
    618: {"name": "Winterspring", "map": 1, "x": 6724.0, "y": -4606.0, "z": 721.0, "level_range": (53, 60)},
    1377: {"name": "Silithus", "map": 1, "x": -6815.0, "y": 730.0, "z": 42.0, "level_range": (55, 60)},
    490: {"name": "Un'Goro Crater", "map": 1, "x": -6204.0, "y": -1061.0, "z": -217.0, "level_range": (48, 55)},
    405: {"name": "Desolace", "map": 1, "x": -1573.0, "y": 2860.0, "z": 76.0, "level_range": (30, 40)},
    406: {"name": "Stonetalon Mountains", "map": 1, "x": 920.0, "y": 920.0, "z": 104.0, "level_range": (25, 35)},
}

# Horde bot GUIDs (from step 4)
HORDE_BOT_GUIDS = list(range(33, 53))  # GUIDs 33-52
# Alliance city bot GUIDs
ALLIANCE_CITY_GUIDS = list(range(53, 73))  # GUIDs 53-72
# Alliance world bot GUIDs
ALLIANCE_WORLD_GUIDS = list(range(73, 93))  # GUIDs 73-92


def get_db():
    config = DB_CONFIG.copy()
    config["database"] = "characters"
    return mysql.connector.connect(**config)


def get_online_players():
    """Get real online players (not bots)."""
    conn = get_db()
    cursor = conn.cursor(dictionary=True)
    cursor.execute("""
        SELECT c.guid, c.name, c.level, c.race, c.class as cls, c.zone, c.map,
               c.position_x, c.position_y, c.position_z
        FROM characters c
        WHERE c.online = 1 AND c.account < 100
    """)
    players = cursor.fetchall()
    cursor.close()
    conn.close()
    return players


def classify_zone(zone_id):
    """Classify a zone as capital, friendly, or contested."""
    if zone_id in CAPITAL_CITIES:
        return "capital", CAPITAL_CITIES[zone_id]
    if zone_id in FRIENDLY_ZONES:
        return "friendly", FRIENDLY_ZONES[zone_id]
    if zone_id in CONTESTED_ZONES:
        return "contested", CONTESTED_ZONES[zone_id]
    # Unknown zone — treat as friendly
    return "friendly", None


def assign_bots(players):
    """Main director logic — assign bots based on player locations."""
    assignments = []

    if not players:
        logging.info("No players online — clearing assignments")
        clear_all_assignments()
        return

    # Categorize players
    city_players = []
    friendly_players = []
    contested_players = []

    for p in players:
        zone_type, zone_info = classify_zone(p["zone"])
        if zone_type == "capital":
            city_players.append((p, zone_info))
        elif zone_type == "contested":
            contested_players.append((p, zone_info))
        else:
            friendly_players.append((p, zone_info))

    # Track which bot GUIDs we've assigned
    used_horde = []
    used_city = []
    used_world = []
    available_horde = list(HORDE_BOT_GUIDS)
    available_city = list(ALLIANCE_CITY_GUIDS)
    available_world = list(ALLIANCE_WORLD_GUIDS)
    random.shuffle(available_horde)
    random.shuffle(available_city)
    random.shuffle(available_world)

    # Always keep some city bots in Stormwind and Ironforge for atmosphere
    cities = [
        (1519, CAPITAL_CITIES[1519]),  # Stormwind
        (1537, CAPITAL_CITIES[1537]),  # Ironforge
    ]
    for zone_id, zone_info in cities:
        num_bots = min(5, len(available_city))
        for i in range(num_bots):
            guid = available_city.pop(0)
            if random.random() < 0.4:
                level = 60
            elif random.random() < 0.3:
                level = random.randint(40, 59)
            else:
                level = random.randint(10, 39)
            assignments.append({
                "guid": guid,
                "target_map": zone_info["map"],
                "target_x": zone_info["x"],
                "target_y": zone_info["y"],
                "target_z": zone_info["z"],
                "target_zone": zone_id,
                "radius": 80,
                "behavior": "passive",
                "target_level": level,
                "faction": 469,
            })
            used_city.append(guid)
        logging.info(f"  Always-on: {num_bots} city bots in {zone_info['name']}")

    # === CAPITAL CITY: add extra bots if player is in a city ===
    for player, zone_info in city_players:
        if not zone_info:
            continue
        # Add extra bots beyond the always-on ones
        num_bots = min(5, len(available_city))
        for i in range(num_bots):
            guid = available_city.pop(0)
            if random.random() < 0.4:
                level = 60
            elif random.random() < 0.3:
                level = random.randint(40, 59)
            else:
                level = random.randint(10, 39)
            assignments.append({
                "guid": guid,
                "target_map": zone_info["map"],
                "target_x": zone_info["x"],
                "target_y": zone_info["y"],
                "target_z": zone_info["z"],
                "target_zone": [k for k, v in CAPITAL_CITIES.items() if v == zone_info][0],
                "radius": 80,
                "behavior": "passive",
                "target_level": level,
                "faction": 469,
            })
            used_city.append(guid)
        logging.info(f"  Capital {zone_info['name']}: extra {num_bots} city bots for {player['name']}")

    # === FRIENDLY ZONE: alliance bots near player level ===
    for player, zone_info in friendly_players:
        if not zone_info:
            continue
        plevel = player["level"]
        num_bots = min(5, len(available_world))
        for i in range(num_bots):
            guid = available_world.pop(0)
            level = max(1, min(60, plevel + random.randint(-5, 5)))
            assignments.append({
                "guid": guid,
                "target_map": zone_info["map"],
                "target_x": zone_info["x"],
                "target_y": zone_info["y"],
                "target_z": zone_info["z"],
                "target_zone": [k for k, v in FRIENDLY_ZONES.items() if v == zone_info][0],
                "radius": 150,
                "behavior": "passive",
                "target_level": level,
                "faction": 469,  # Alliance
            })
            used_world.append(guid)
        logging.info(f"  Friendly {zone_info['name']}: assigned {num_bots} world bots for {player['name']} (lvl {plevel})")

    # === CONTESTED ZONE: both factions, enemies aggressive ===
    for player, zone_info in contested_players:
        if not zone_info:
            continue
        plevel = player["level"]

        # Assign 3-5 horde gankers
        num_horde = min(5, len(available_horde))
        for i in range(num_horde):
            guid = available_horde.pop(0)
            # 10% chance of level 60 for tension
            if random.random() < 0.1:
                level = 60
                logging.info(f"    !! Level 60 Horde ganker spawning near {player['name']}!")
            else:
                level = max(1, min(60, plevel + random.randint(-5, 5)))
            assignments.append({
                "guid": guid,
                "target_map": zone_info["map"],
                "target_x": zone_info["x"],
                "target_y": zone_info["y"],
                "target_z": zone_info["z"],
                "target_zone": [k for k, v in CONTESTED_ZONES.items() if v == zone_info][0],
                "radius": 200,
                "behavior": "aggressive",
                "target_level": level,
                "faction": 67,  # Horde
            })
            used_horde.append(guid)

        # Assign 2-3 friendly alliance bots
        num_friendly = min(3, len(available_world))
        for i in range(num_friendly):
            guid = available_world.pop(0)
            # 10% chance of level 60 friendly for relief
            if random.random() < 0.1:
                level = 60
                logging.info(f"    ** Level 60 Alliance hero near {player['name']}!")
            else:
                level = max(1, min(60, plevel + random.randint(-5, 5)))
            assignments.append({
                "guid": guid,
                "target_map": zone_info["map"],
                "target_x": zone_info["x"],
                "target_y": zone_info["y"],
                "target_z": zone_info["z"],
                "target_zone": [k for k, v in CONTESTED_ZONES.items() if v == zone_info][0],
                "radius": 200,
                "behavior": "passive",
                "target_level": level,
                "faction": 469,  # Alliance
            })
            used_world.append(guid)

        logging.info(f"  Contested {zone_info['name']}: {num_horde} horde + {num_friendly} alliance for {player['name']} (lvl {plevel})")

    # Write assignments to DB
    write_assignments(assignments)

    # Update chance weights — assigned bots get high chance, unassigned get 0
    assigned_guids = [a["guid"] for a in assignments]
    update_chances(assigned_guids)

    logging.info(f"Director cycle complete: {len(assignments)} bots assigned for {len(players)} players")


def write_assignments(assignments):
    """Write bot assignments to bot_assignments and update characters table directly."""
    conn = get_db()
    cursor = conn.cursor()

    # Clear old assignments
    cursor.execute("DELETE FROM bot_assignments")

    for a in assignments:
        cursor.execute("""
            INSERT INTO bot_assignments
                (guid, target_map, target_x, target_y, target_z, target_zone, radius, behavior, target_level, faction)
            VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
            ON DUPLICATE KEY UPDATE
                target_map=%s, target_x=%s, target_y=%s, target_z=%s, target_zone=%s,
                radius=%s, behavior=%s, target_level=%s, faction=%s
        """, (
            a["guid"], a["target_map"], a["target_x"], a["target_y"], a["target_z"],
            a["target_zone"], a["radius"], a["behavior"], a["target_level"], a["faction"],
            a["target_map"], a["target_x"], a["target_y"], a["target_z"],
            a["target_zone"], a["radius"], a["behavior"], a["target_level"], a["faction"],
        ))

        # Also update the characters table so bots spawn at the right location
        # Add some randomness to position
        import random as rnd
        offset_x = rnd.uniform(-a["radius"] * 0.3, a["radius"] * 0.3)
        offset_y = rnd.uniform(-a["radius"] * 0.3, a["radius"] * 0.3)
        cursor.execute("""
            UPDATE characters SET
                map = %s,
                position_x = %s,
                position_y = %s,
                position_z = %s,
                zone = %s,
                level = %s
            WHERE guid = %s
        """, (
            a["target_map"],
            a["target_x"] + offset_x,
            a["target_y"] + offset_y,
            a["target_z"],
            a["target_zone"],
            a["target_level"],
            a["guid"],
        ))

    conn.commit()
    cursor.close()
    conn.close()


def update_chances(assigned_guids):
    """Update chance weights so assigned bots spawn preferentially."""
    conn = get_db()
    cursor = conn.cursor()

    all_dynamic = HORDE_BOT_GUIDS + ALLIANCE_CITY_GUIDS + ALLIANCE_WORLD_GUIDS

    for guid in all_dynamic:
        if guid in assigned_guids:
            cursor.execute("UPDATE playerbot SET chance = 30 WHERE char_guid = %s", (guid,))
        else:
            cursor.execute("UPDATE playerbot SET chance = 1 WHERE char_guid = %s", (guid,))

    conn.commit()
    cursor.close()
    conn.close()


def clear_all_assignments():
    """Clear all assignments when no players are online."""
    conn = get_db()
    cursor = conn.cursor()
    cursor.execute("DELETE FROM bot_assignments")
    # Set all chances low
    all_dynamic = HORDE_BOT_GUIDS + ALLIANCE_CITY_GUIDS + ALLIANCE_WORLD_GUIDS
    for guid in all_dynamic:
        cursor.execute("UPDATE playerbot SET chance = 1 WHERE char_guid = %s", (guid,))
    conn.commit()
    cursor.close()
    conn.close()


def main():
    logging.info("Bot Director starting...")
    while True:
        try:
            players = get_online_players()
            logging.info(f"--- Director cycle: {len(players)} players online ---")
            assign_bots(players)
        except Exception as e:
            logging.error(f"Director error: {e}")
        time.sleep(120)  # Run every 2 minutes


if __name__ == "__main__":
    main()
