#!/usr/bin/env python3
"""Extract zone-as-data for leveling bots from the world database.

See docs/BOT_LEVELING_DESIGN.md ("Zone-as-data"). Reads the world DB
(quest templates, quest relations, creature/gameobject spawns, loot
tables) and emits one JSON document per zone: the quest graph with
classified objectives, clustered hotspots for kill/collect targets,
hub NPCs with services, and grind-spot candidates. This JSON is the
planner/skills input; adding a zone should be new output from this
extractor, not new AI code.

Quest membership: quest_template.ZoneOrSort == zone id. Spawns have no
zone column, so in-zone spawn membership uses a per-zone coordinate
bounding box on the zone's map. The Silverpine defaults below were fit
empirically (they cover 58/58 objective-creature spawns of the zone's
quests; see the design doc). Quests whose starter or turn-in only spawns
outside the box are classified as needing review for a zone-local run;
their cross-zone endpoints remain visible in the output.

Objectives are classified into skill-shaped kinds (kill, collect_drop,
collect_object, interact_go, explore, talk). Anything the skill library
cannot execute yet is flagged in `classification.reasons` (spellcast
requirements, unknown item sources, unclassifiable objectives) so the
planner can skip it instead of wedging.

Credentials come from the environment (DB_HOST/DB_PORT/DB_USER/
DB_PASSWORD, matching scripts/backup-db.sh). The password reaches the
mysql client via MYSQL_PWD, never via argv.
"""

from __future__ import annotations

import argparse
import json
import logging
import math
import os
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

LOG = logging.getLogger("extract_zone_data")

SCHEMA_VERSION = 1

# npc_flags bits used for hub service detection (vanilla values).
NPC_FLAG_VENDOR = 0x04
NPC_FLAG_REPAIR = 0x1000
NPC_FLAG_FLIGHTMASTER = 0x08
NPC_FLAG_INNKEEPER = 0x80
NPC_FLAG_TRAINER = 0x10
NPC_FLAG_BANKER = 0x020000
NPC_FLAG_AUCTIONEER = 0x200000

SERVICES = {
    "vendor": NPC_FLAG_VENDOR,
    "repair": NPC_FLAG_REPAIR,
    "flightmaster": NPC_FLAG_FLIGHTMASTER,
    "innkeeper": NPC_FLAG_INNKEEPER,
    "trainer": NPC_FLAG_TRAINER,
    "banker": NPC_FLAG_BANKER,
    "auctioneer": NPC_FLAG_AUCTIONEER,
}

# QuestTemplate "Type" values worth flagging for the planner.
GROUP_TYPES = {1: "elite", 62: "raid", 81: "dungeon"}


@dataclass
class DbConfig:
    host: str
    port: int
    user: str
    password: str
    world_db: str


def load_db_config(args: argparse.Namespace) -> DbConfig:
    password = os.environ.get("DB_PASSWORD")
    if password is None:
        raise SystemExit("DB_PASSWORD is not set (export it first; empty string is accepted for local dev)")
    return DbConfig(
        host=os.environ.get("DB_HOST", "127.0.0.1"),
        port=int(os.environ.get("DB_PORT", "3306")),
        user=os.environ.get("DB_USER", "mangos"),
        password=password,
        world_db=args.world_db,
    )


def sql_rows(config: DbConfig, sql: str) -> list[list[str]]:
    """Run a SELECT through the mysql client; returns rows of strings."""
    env = dict(os.environ)
    env["MYSQL_PWD"] = config.password
    argv = [
        "mysql",
        "--batch",
        "--raw",
        "--skip-column-names",
        "-h",
        config.host,
        "-P",
        str(config.port),
        "-u",
        config.user,
        config.world_db,
        "-e",
        sql,
    ]
    result = subprocess.run(argv, capture_output=True, text=True, env=env, timeout=600)
    if result.returncode != 0:
        raise RuntimeError(f"mysql failed: {result.stderr.strip()}\nSQL: {sql[:500]}")
    return [line.split("\t") for line in result.stdout.splitlines() if line]


def as_int(value: str, default: int = 0) -> int:
    try:
        return int(float(value))
    except (TypeError, ValueError):
        return default


def as_float(value: str, default: float = 0.0) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


@dataclass
class Point:
    x: float
    y: float
    z: float

    def rounded(self) -> dict[str, float]:
        return {"x": round(self.x, 2), "y": round(self.y, 2), "z": round(self.z, 2)}


def cluster_points(points: list[Point], cell_yards: float = 100.0) -> list[dict]:
    """Grid-cluster spawn points into hotspots (centroid, count, radius)."""
    cells: dict[tuple[int, int], list[Point]] = defaultdict(list)
    for point in points:
        cells[(math.floor(point.x / cell_yards), math.floor(point.y / cell_yards))].append(point)
    hotspots = []
    for members in cells.values():
        centroid = Point(
            sum(p.x for p in members) / len(members),
            sum(p.y for p in members) / len(members),
            sum(p.z for p in members) / len(members),
        )
        radius = max(math.hypot(p.x - centroid.x, p.y - centroid.y) for p in members)
        hotspots.append({**centroid.rounded(), "spawn_count": len(members), "radius_yards": round(radius, 1)})
    hotspots.sort(key=lambda h: h["spawn_count"], reverse=True)
    return hotspots


QUEST_COLUMNS = [
    "entry",
    "ZoneOrSort",
    "MinLevel",
    "QuestLevel",
    "Type",
    "RequiredClasses",
    "RequiredRaces",
    "SuggestedPlayers",
    "LimitTime",
    "QuestFlags",
    "SpecialFlags",
    "PrevQuestId",
    "NextQuestId",
    "ExclusiveGroup",
    "BreadcrumbForQuestId",
    "NextQuestInChain",
    "SrcItemId",
    "Title",
    "ReqItemId1",
    "ReqItemId2",
    "ReqItemId3",
    "ReqItemId4",
    "ReqItemCount1",
    "ReqItemCount2",
    "ReqItemCount3",
    "ReqItemCount4",
    "ReqCreatureOrGOId1",
    "ReqCreatureOrGOId2",
    "ReqCreatureOrGOId3",
    "ReqCreatureOrGOId4",
    "ReqCreatureOrGOCount1",
    "ReqCreatureOrGOCount2",
    "ReqCreatureOrGOCount3",
    "ReqCreatureOrGOCount4",
    "ReqSpellCast1",
    "ReqSpellCast2",
    "ReqSpellCast3",
    "ReqSpellCast4",
    "RewChoiceItemId1",
    "RewChoiceItemId2",
    "RewChoiceItemId3",
    "RewItemId1",
    "RewItemId2",
    "RewXP",
    "RewOrReqMoney",
    "RewSpell",
]


@dataclass
class ZoneData:
    quests: dict[int, dict[str, str]]
    starters_creature: dict[int, list[int]]  # quest -> creature entries
    finishers_creature: dict[int, list[int]]
    starters_go: dict[int, list[int]]
    finishers_go: dict[int, list[int]]
    explore_triggers: dict[int, list[int]]  # quest -> areatrigger ids
    scripted_starts: set[int]
    creature_templates: dict[int, dict]
    go_templates: dict[int, dict]
    item_names: dict[int, str]
    creature_spawns: dict[int, list[Point]]  # entry -> in-zone spawns
    go_spawns: dict[int, list[Point]]
    item_sources: dict[int, dict]  # item -> {"creature": [entries], "gameobject": [entries]}


def relation_map(rows: list[list[str]]) -> dict[int, list[int]]:
    out: dict[int, list[int]] = defaultdict(list)
    for npc_id, quest in rows:
        out[as_int(quest)].append(as_int(npc_id))
    return out


def load_zone_data(config: DbConfig, zone_id: int, bbox: tuple[float, float, float, float], map_id: int) -> ZoneData:
    min_x, max_x, min_y, max_y = bbox
    in_box = f"map = {map_id} AND position_x BETWEEN {min_x} AND {max_x} AND position_y BETWEEN {min_y} AND {max_y}"

    quest_rows = sql_rows(
        config,
        f"SELECT {', '.join(QUEST_COLUMNS)} FROM quest_template WHERE ZoneOrSort = {zone_id} ORDER BY entry",
    )
    quests = {as_int(row[0]): dict(zip(QUEST_COLUMNS, row)) for row in quest_rows}
    quest_ids = ", ".join(str(q) for q in quests) or "0"

    starters_creature = relation_map(
        sql_rows(config, f"SELECT id, quest FROM creature_questrelation WHERE quest IN ({quest_ids})")
    )
    finishers_creature = relation_map(
        sql_rows(config, f"SELECT id, quest FROM creature_involvedrelation WHERE quest IN ({quest_ids})")
    )
    starters_go = relation_map(
        sql_rows(config, f"SELECT id, quest FROM gameobject_questrelation WHERE quest IN ({quest_ids})")
    )
    finishers_go = relation_map(
        sql_rows(config, f"SELECT id, quest FROM gameobject_involvedrelation WHERE quest IN ({quest_ids})")
    )
    explore_triggers = relation_map(
        sql_rows(config, f"SELECT id, quest FROM areatrigger_involvedrelation WHERE quest IN ({quest_ids})")
    )
    scripted_starts = {
        as_int(row[0])
        for row in sql_rows(config, f"SELECT DISTINCT id FROM quest_start_scripts WHERE id IN ({quest_ids})")
    }

    # Creature entries we need: quest givers, finishers, kill objectives.
    creature_ids: set[int] = set()
    for mapping in (starters_creature, finishers_creature):
        for entries in mapping.values():
            creature_ids.update(entries)
    item_ids: set[int] = set()
    go_ids: set[int] = set()
    for quest in quests.values():
        for slot in range(1, 5):
            creature_or_go = as_int(quest[f"ReqCreatureOrGOId{slot}"])
            if creature_or_go > 0:
                creature_ids.add(creature_or_go)
            elif creature_or_go < 0:
                go_ids.add(-creature_or_go)
            item_id = as_int(quest[f"ReqItemId{slot}"])
            if item_id:
                item_ids.add(item_id)
    for mapping in (starters_go, finishers_go):
        for entries in mapping.values():
            go_ids.update(entries)

    creature_templates: dict[int, dict] = {}
    if creature_ids:
        ids = ", ".join(str(i) for i in sorted(creature_ids))
        for row in sql_rows(
            config,
            f"SELECT entry, name, level_min, level_max, npc_flags FROM creature_template WHERE entry IN ({ids})",
        ):
            creature_templates[as_int(row[0])] = {
                "name": row[1],
                "min_level": as_int(row[2]),
                "max_level": as_int(row[3]),
                "npc_flags": as_int(row[4]),
            }
    go_templates: dict[int, dict] = {}
    if go_ids:
        ids = ", ".join(str(i) for i in sorted(go_ids))
        for row in sql_rows(config, f"SELECT entry, type, name FROM gameobject_template WHERE entry IN ({ids})"):
            go_templates[as_int(row[0])] = {"name": row[2], "type": as_int(row[1])}
    item_names: dict[int, str] = {}
    if item_ids:
        ids = ", ".join(str(i) for i in sorted(item_ids))
        for row in sql_rows(config, f"SELECT entry, name FROM item_template WHERE entry IN ({ids})"):
            item_names[as_int(row[0])] = row[1]

    creature_spawns: dict[int, list[Point]] = defaultdict(list)
    if creature_ids:
        ids = ", ".join(str(i) for i in sorted(creature_ids))
        for row in sql_rows(
            config,
            f"SELECT id, position_x, position_y, position_z FROM creature WHERE id IN ({ids}) AND {in_box}",
        ):
            creature_spawns[as_int(row[0])].append(Point(as_float(row[1]), as_float(row[2]), as_float(row[3])))
    go_spawns: dict[int, list[Point]] = defaultdict(list)
    if go_ids:
        ids = ", ".join(str(i) for i in sorted(go_ids))
        for row in sql_rows(
            config,
            f"SELECT id, position_x, position_y, position_z FROM gameobject WHERE id IN ({ids}) AND {in_box}",
        ):
            go_spawns[as_int(row[0])].append(Point(as_float(row[1]), as_float(row[2]), as_float(row[3])))

    item_sources: dict[int, dict] = {item: {"creature": [], "gameobject": []} for item in item_ids}
    if item_ids:
        # Loot-table `entry` values are loot template ids, not necessarily
        # spawnable creature/gameobject entries. Collect them first, then
        # resolve through creature_template.loot_id and chest data1 below.
        loot_ids_by_item: dict[int, dict[str, list[int]]] = {
            item: {"creature": [], "gameobject": []} for item in item_ids
        }
        ids = ", ".join(str(i) for i in sorted(item_ids))
        for row in sql_rows(config, f"SELECT item, entry FROM creature_loot_template WHERE item IN ({ids})"):
            loot_ids_by_item[as_int(row[0])]["creature"].append(as_int(row[1]))
        for row in sql_rows(config, f"SELECT item, entry FROM gameobject_loot_template WHERE item IN ({ids})"):
            loot_ids_by_item[as_int(row[0])]["gameobject"].append(as_int(row[1]))
        # Reference-loot indirection: item -> reference template ids ->
        # loot rows whose mincountOrRef = -reference_id.
        ref_ids_by_item: dict[int, list[int]] = defaultdict(list)
        for row in sql_rows(config, f"SELECT item, entry FROM reference_loot_template WHERE item IN ({ids})"):
            ref_ids_by_item[as_int(row[0])].append(as_int(row[1]))
        if ref_ids_by_item:
            ref_to_items: dict[int, list[int]] = defaultdict(list)
            for item, refs in ref_ids_by_item.items():
                for ref in refs:
                    ref_to_items[ref].append(item)
            neg_refs = ", ".join(str(-ref) for ref in sorted(ref_to_items))
            for table, key in (("creature_loot_template", "creature"), ("gameobject_loot_template", "gameobject")):
                for row in sql_rows(
                    config, f"SELECT entry, mincountOrRef FROM {table} WHERE mincountOrRef IN ({neg_refs})"
                ):
                    for item in ref_to_items[-as_int(row[1])]:
                        loot_ids_by_item[item][key].append(as_int(row[0]))
        # Resolve loot template ids to spawnable template entries. The
        # legacy convention loot_id == entry is kept as a fallback.
        creature_loot_ids = sorted({loot for src in loot_ids_by_item.values() for loot in src["creature"]})
        if creature_loot_ids:
            loot_set = set(creature_loot_ids)
            ids = ", ".join(str(i) for i in creature_loot_ids)
            resolved: dict[int, list[int]] = defaultdict(list)
            for row in sql_rows(
                config,
                f"SELECT entry, loot_id FROM creature_template WHERE entry IN ({ids}) OR loot_id IN ({ids})",
            ):
                entry, loot_id = as_int(row[0]), as_int(row[1])
                for key in {entry, loot_id} & loot_set:
                    if entry not in resolved[key]:
                        resolved[key].append(entry)
            for item, src in loot_ids_by_item.items():
                entries = {entry for loot in src["creature"] for entry in resolved.get(loot, [])}
                item_sources[item]["creature"] = sorted(entries)
        go_loot_ids = sorted({loot for src in loot_ids_by_item.values() for loot in src["gameobject"]})
        if go_loot_ids:
            loot_set = set(go_loot_ids)
            ids = ", ".join(str(i) for i in go_loot_ids)
            resolved_go: dict[int, list[int]] = defaultdict(list)
            for row in sql_rows(
                config,
                "SELECT entry, type, data1 FROM gameobject_template"
                f" WHERE entry IN ({ids}) OR (type = 3 AND data1 IN ({ids}))",
            ):
                entry, go_type, data1 = as_int(row[0]), as_int(row[1]), as_int(row[2])
                keys = {entry} if entry in loot_set else set()
                if go_type == 3 and data1 in loot_set:
                    keys.add(data1)
                for key in keys:
                    if entry not in resolved_go[key]:
                        resolved_go[key].append(entry)
            for item, src in loot_ids_by_item.items():
                entries = {entry for loot in src["gameobject"] for entry in resolved_go.get(loot, [])}
                item_sources[item]["gameobject"] = sorted(entries)
        # Loot-source gameobjects may add hotspots for collect objectives.
        extra_gos = {e for src in item_sources.values() for e in src["gameobject"]} - set(go_spawns)
        if extra_gos:
            ids = ", ".join(str(i) for i in sorted(extra_gos))
            for row in sql_rows(config, f"SELECT entry, type, name FROM gameobject_template WHERE entry IN ({ids})"):
                go_templates.setdefault(as_int(row[0]), {"name": row[2], "type": as_int(row[1])})
            for row in sql_rows(
                config,
                f"SELECT id, position_x, position_y, position_z FROM gameobject WHERE id IN ({ids}) AND {in_box}",
            ):
                go_spawns[as_int(row[0])].append(Point(as_float(row[1]), as_float(row[2]), as_float(row[3])))
        # Drop-source creatures may add hotspots for collect objectives.
        extra_creatures = {e for src in item_sources.values() for e in src["creature"]} - set(creature_spawns)
        if extra_creatures:
            ids = ", ".join(str(i) for i in sorted(extra_creatures))
            for row in sql_rows(
                config,
                f"SELECT entry, name, level_min, level_max, npc_flags FROM creature_template WHERE entry IN ({ids})",
            ):
                creature_templates.setdefault(
                    as_int(row[0]),
                    {
                        "name": row[1],
                        "min_level": as_int(row[2]),
                        "max_level": as_int(row[3]),
                        "npc_flags": as_int(row[4]),
                    },
                )
            for row in sql_rows(
                config,
                f"SELECT id, position_x, position_y, position_z FROM creature WHERE id IN ({ids}) AND {in_box}",
            ):
                creature_spawns[as_int(row[0])].append(Point(as_float(row[1]), as_float(row[2]), as_float(row[3])))

    return ZoneData(
        quests=quests,
        starters_creature=starters_creature,
        finishers_creature=finishers_creature,
        starters_go=starters_go,
        finishers_go=finishers_go,
        explore_triggers=explore_triggers,
        scripted_starts=scripted_starts,
        creature_templates=creature_templates,
        go_templates=go_templates,
        item_names=item_names,
        creature_spawns=creature_spawns,
        go_spawns=go_spawns,
        item_sources=item_sources,
    )


def endpoint(kind: str, entry_id: int, data: ZoneData) -> dict:
    """Quest start/end reference: template name + in-zone spawn points."""
    if kind == "npc":
        template = data.creature_templates.get(entry_id, {})
        spawns = data.creature_spawns.get(entry_id, [])
        return {"type": "npc", "id": entry_id, "name": template.get("name"), "spawns": [p.rounded() for p in spawns]}
    template = data.go_templates.get(entry_id, {})
    spawns = data.go_spawns.get(entry_id, [])
    return {"type": "gameobject", "id": entry_id, "name": template.get("name"), "spawns": [p.rounded() for p in spawns]}


def endpoints(quest_id: int, creature_map: dict, go_map: dict, data: ZoneData) -> list[dict]:
    out = [endpoint("npc", entry, data) for entry in creature_map.get(quest_id, [])]
    out += [endpoint("gameobject", entry, data) for entry in go_map.get(quest_id, [])]
    return out


def classify_objectives(quest_id: int, quest: dict[str, str], data: ZoneData) -> tuple[list[dict], list[str]]:
    """Translate raw requirements into skill-shaped objectives + skip reasons."""
    objectives: list[dict] = []
    reasons: list[str] = []
    for slot in range(1, 5):
        creature_or_go = as_int(quest[f"ReqCreatureOrGOId{slot}"])
        count = as_int(quest[f"ReqCreatureOrGOCount{slot}"])
        if creature_or_go > 0:
            template = data.creature_templates.get(creature_or_go, {})
            spawns = data.creature_spawns.get(creature_or_go, [])
            objective = {
                "kind": "kill",
                "creature_id": creature_or_go,
                "name": template.get("name"),
                "creature_level": [template.get("min_level"), template.get("max_level")],
                "count": count,
                "hotspots": cluster_points(spawns),
            }
            if not spawns:
                objective["warning"] = "no in-zone spawns found"
                reasons.append(f"kill target {creature_or_go} has no in-zone spawns")
            objectives.append(objective)
        elif creature_or_go < 0:
            go_id = -creature_or_go
            template = data.go_templates.get(go_id, {})
            objectives.append(
                {
                    "kind": "interact_go",
                    "gameobject_id": go_id,
                    "name": template.get("name"),
                    "count": count,
                    "hotspots": cluster_points(data.go_spawns.get(go_id, [])),
                }
            )
        item_id = as_int(quest[f"ReqItemId{slot}"])
        if item_id and item_id == as_int(quest["SrcItemId"]):
            # Courier quest: the quest hands over the very item it wants
            # delivered. Executable as travel + talk; no farming needed.
            objectives.append(
                {
                    "kind": "deliver",
                    "item_id": item_id,
                    "name": data.item_names.get(item_id),
                    "count": as_int(quest[f"ReqItemCount{slot}"]),
                }
            )
            continue
        if item_id:
            sources = data.item_sources.get(item_id, {"creature": [], "gameobject": []})
            hotspots: list[dict] = []
            if sources["creature"]:
                kind = "collect_drop"
                for entry in sources["creature"]:
                    hotspots += cluster_points(data.creature_spawns.get(entry, []))
            elif sources["gameobject"]:
                kind = "collect_object"
                for entry in sources["gameobject"]:
                    hotspots += cluster_points(data.go_spawns.get(entry, []))
            else:
                kind = "collect_unknown"
                reasons.append(f"item {item_id} has no creature/gameobject loot source")
            if kind in ("collect_drop", "collect_object") and not hotspots:
                reasons.append(
                    f"item {item_id} sources have no in-zone static spawns"
                    " (script/pool-spawned or outside the zone bbox)"
                )
            objectives.append(
                {
                    "kind": kind,
                    "item_id": item_id,
                    "name": data.item_names.get(item_id),
                    "count": as_int(quest[f"ReqItemCount{slot}"]),
                    "source_creatures": sources["creature"],
                    "source_gameobjects": sources["gameobject"],
                    "hotspots": hotspots,
                }
            )
        spell_cast = as_int(quest[f"ReqSpellCast{slot}"])
        if spell_cast:
            objectives.append({"kind": "spellcast", "spell_id": spell_cast})
            reasons.append(f"requires spell cast {spell_cast} (no skill covers targeted spell objectives yet)")
    for trigger_id in data.explore_triggers.get(quest_id, []):
        objectives.append({"kind": "explore", "areatrigger_id": trigger_id})
        reasons.append(f"explore trigger {trigger_id} coordinates live in AreaTrigger.dbc, not the world DB")
    if not objectives:
        if data.finishers_creature.get(quest_id) or data.finishers_go.get(quest_id):
            objectives.append({"kind": "talk"})
        else:
            reasons.append("no classifiable objective and no end NPC/GO")
    if quest_id in data.scripted_starts:
        reasons.append("has quest_start_scripts entry (may be escort/scripted; verify before enabling)")
    # Note: a bare SrcItemId is not a skip reason. Courier quests
    # (ReqItemId == SrcItemId) are classified as `deliver` above, and
    # other source items are usually passive; genuinely used items tend
    # to also carry a ReqSpellCast or scripted start, flagged separately.
    if as_int(quest["LimitTime"]):
        reasons.append("timed quest")
    if as_int(quest["SuggestedPlayers"]) > 1:
        reasons.append(f"suggested players {quest['SuggestedPlayers']}")
    if as_int(quest["Type"]) in GROUP_TYPES:
        reasons.append(f"quest type {GROUP_TYPES[as_int(quest['Type'])]}")
    return objectives, reasons


def build_document(config: DbConfig, args: argparse.Namespace, data: ZoneData) -> dict:
    quests_out = []
    out_of_zone_refs: set[int] = set()
    for quest_id in sorted(data.quests):
        quest = data.quests[quest_id]
        objectives, reasons = classify_objectives(quest_id, quest, data)
        start_points = endpoints(quest_id, data.starters_creature, data.starters_go, data)
        end_points = endpoints(quest_id, data.finishers_creature, data.finishers_go, data)
        if not start_points:
            reasons.append("no known quest starter (item start or missing relation)")
        elif not any(endpoint.get("spawns") for endpoint in start_points):
            reasons.append("quest starter has no in-zone spawn (starts outside zone)")
        if end_points and not any(endpoint.get("spawns") for endpoint in end_points):
            reasons.append("quest turn-in has no in-zone spawn (ends outside zone)")
        for field in ("PrevQuestId", "NextQuestId", "NextQuestInChain", "BreadcrumbForQuestId"):
            ref = abs(as_int(quest[field]))
            if ref and ref not in data.quests:
                out_of_zone_refs.add(ref)
        quests_out.append(
            {
                "id": quest_id,
                "title": quest["Title"],
                "quest_level": as_int(quest["QuestLevel"]),
                "min_level": as_int(quest["MinLevel"]),
                "type": as_int(quest["Type"]),
                "suggested_players": as_int(quest["SuggestedPlayers"]),
                "required_races_mask": as_int(quest["RequiredRaces"]),
                "required_classes_mask": as_int(quest["RequiredClasses"]),
                "prev_quest": as_int(quest["PrevQuestId"]),
                "next_quest": as_int(quest["NextQuestId"]),
                "next_in_chain": as_int(quest["NextQuestInChain"]),
                "exclusive_group": as_int(quest["ExclusiveGroup"]),
                "breadcrumb_for": as_int(quest["BreadcrumbForQuestId"]),
                "start": start_points,
                "end": end_points,
                "objectives": objectives,
                "rewards": {
                    "xp_column": as_int(quest["RewXP"]),
                    "money_copper": as_int(quest["RewOrReqMoney"]),
                    "spell": as_int(quest["RewSpell"]),
                    "choice_items": [
                        as_int(quest[f"RewChoiceItemId{i}"]) for i in (1, 2, 3) if as_int(quest[f"RewChoiceItemId{i}"])
                    ],
                    "items": [as_int(quest[f"RewItemId{i}"]) for i in (1, 2) if as_int(quest[f"RewItemId{i}"])],
                },
                "classification": {
                    "implementable": not reasons,
                    "reasons": reasons,
                },
            }
        )

    # Hubs: quest starter/finisher NPCs with in-zone spawns, clustered.
    hub_npcs: list[dict] = []
    npc_ids = set()
    for mapping in (data.starters_creature, data.finishers_creature):
        for entries in mapping.values():
            npc_ids.update(entries)
    npc_points: list[tuple[int, Point]] = []
    for npc_id in sorted(npc_ids):
        template = data.creature_templates.get(npc_id, {})
        flags = template.get("npc_flags") or 0
        services = [name for name, bit in SERVICES.items() if flags & bit]
        starts = sorted(q for q, entries in data.starters_creature.items() if npc_id in entries)
        ends = sorted(q for q, entries in data.finishers_creature.items() if npc_id in entries)
        spawns = data.creature_spawns.get(npc_id, [])
        hub_npcs.append(
            {
                "npc_id": npc_id,
                "name": template.get("name"),
                "services": services,
                "starts_quests": starts,
                "ends_quests": ends,
                "spawns": [p.rounded() for p in spawns],
            }
        )
        npc_points += [(npc_id, p) for p in spawns]
    hubs = []
    if npc_points:
        cells: dict[tuple[int, int], list[tuple[int, Point]]] = defaultdict(list)
        for npc_id, point in npc_points:
            cells[(math.floor(point.x / 60.0), math.floor(point.y / 60.0))].append((npc_id, point))
        for index, members in enumerate(sorted(cells.values(), key=len, reverse=True), start=1):
            centroid = Point(
                sum(p.x for _, p in members) / len(members),
                sum(p.y for _, p in members) / len(members),
                sum(p.z for _, p in members) / len(members),
            )
            hubs.append(
                {
                    "id": index,
                    "center": centroid.rounded(),
                    "npc_ids": sorted({npc_id for npc_id, _ in members}),
                }
            )

    implementable = sum(1 for q in quests_out if q["classification"]["implementable"])
    levels = [q["quest_level"] for q in quests_out if q["quest_level"] > 0]
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_from": {
            "world_db": config.world_db,
            "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "extractor": "scripts/extract_zone_data.py",
        },
        "zone": {
            "id": args.zone_id,
            "name": args.zone_name,
            "map": args.map_id,
            "bbox": {"min_x": args.bbox[0], "max_x": args.bbox[1], "min_y": args.bbox[2], "max_y": args.bbox[3]},
            "quest_level_band": [min(levels) if levels else None, max(levels) if levels else None],
        },
        "summary": {
            "quests": len(quests_out),
            "implementable": implementable,
            "needs_review": len(quests_out) - implementable,
            "quest_npcs": len(hub_npcs),
            "hubs": len(hubs),
            "out_of_zone_quest_refs": sorted(out_of_zone_refs),
        },
        "hubs": hubs,
        "quest_npcs": hub_npcs,
        "quests": quests_out,
    }


def add_grind_spots(config: DbConfig, document: dict, args: argparse.Namespace) -> None:
    """Dense spawn cells by creature level, as grind-filler candidates."""
    min_x, max_x, min_y, max_y = args.bbox
    rows = sql_rows(
        config,
        "SELECT c.id, c.position_x, c.position_y, c.position_z, ct.level_min, ct.level_max, ct.name"
        " FROM creature c JOIN creature_template ct ON ct.entry = c.id"
        f" WHERE c.map = {args.map_id} AND c.position_x BETWEEN {min_x} AND {max_x}"
        f" AND c.position_y BETWEEN {min_y} AND {max_y} AND ct.npc_flags = 0",
    )
    cells: dict[tuple[int, int], list[tuple[int, Point, int, int, str]]] = defaultdict(list)
    for row in rows:
        point = Point(as_float(row[1]), as_float(row[2]), as_float(row[3]))
        cells[(math.floor(point.x / 150.0), math.floor(point.y / 150.0))].append(
            (as_int(row[0]), point, as_int(row[4]), as_int(row[5]), row[6])
        )
    spots = []
    for members in cells.values():
        if len(members) < 8:
            continue
        centroid = Point(
            sum(m[1].x for m in members) / len(members),
            sum(m[1].y for m in members) / len(members),
            sum(m[1].z for m in members) / len(members),
        )
        creatures: dict[int, dict] = {}
        for entry, _, lo, hi, name in members:
            creatures.setdefault(entry, {"creature_id": entry, "name": name, "level": [lo, hi], "spawns": 0})
            creatures[entry]["spawns"] += 1
        spots.append(
            {
                "center": centroid.rounded(),
                "spawn_count": len(members),
                "level_range": [min(m[2] for m in members), max(m[3] for m in members)],
                "creatures": sorted(creatures.values(), key=lambda c: c["spawns"], reverse=True)[:5],
            }
        )
    spots.sort(key=lambda s: s["spawn_count"], reverse=True)
    document["grind_spots"] = spots[:12]
    document["summary"]["grind_spots"] = len(document["grind_spots"])


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--world-db", default="mangos", help="world database name")
    parser.add_argument("--zone-id", type=int, default=130, help="ZoneOrSort id (default: 130 Silverpine)")
    parser.add_argument("--zone-name", default="Silverpine Forest")
    parser.add_argument("--map-id", type=int, default=0)
    parser.add_argument(
        "--bbox",
        type=float,
        nargs=4,
        metavar=("MIN_X", "MAX_X", "MIN_Y", "MAX_Y"),
        default=[-550.0, 1350.0, 600.0, 2150.0],
        help="in-zone spawn bounding box (default: empirically fit Silverpine)",
    )
    parser.add_argument("--out", type=Path, default=Path("data/zones/silverpine.json"))
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=logging.WARNING if args.quiet else logging.INFO, format="%(levelname)s %(name)s: %(message)s"
    )
    config = load_db_config(args)
    data = load_zone_data(config, args.zone_id, tuple(args.bbox), args.map_id)
    if not data.quests:
        raise SystemExit(f"no quests found for ZoneOrSort={args.zone_id} in {config.world_db}")
    document = build_document(config, args, data)
    add_grind_spots(config, document, args)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, indent=2) + "\n")
    summary = document["summary"]
    LOG.info(
        "wrote %s: %d quests (%d implementable, %d need review), %d hubs, %d grind spots",
        args.out,
        summary["quests"],
        summary["implementable"],
        summary["needs_review"],
        summary["hubs"],
        summary["grind_spots"],
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
