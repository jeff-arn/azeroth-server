"""Tests for scripts/extract_zone_data.py — zone-as-data extraction.

Static tests cover clustering and the objective classifier against
synthetic ZoneData (no database). A final test validates the shipped
Silverpine artifact's structure.
"""

import json
from pathlib import Path

from extract_zone_data import Point, ZoneData, classify_objectives, cluster_points

SILVERPINE_JSON = Path(__file__).resolve().parents[2] / "data" / "zones" / "silverpine.json"
BARRENS_JSON = Path(__file__).resolve().parents[2] / "data" / "zones" / "barrens.json"


def make_quest(**overrides):
    quest = {f"ReqItemId{i}": "0" for i in range(1, 5)}
    quest.update({f"ReqItemCount{i}": "0" for i in range(1, 5)})
    quest.update({f"ReqCreatureOrGOId{i}": "0" for i in range(1, 5)})
    quest.update({f"ReqCreatureOrGOCount{i}": "0" for i in range(1, 5)})
    quest.update({f"ReqSpellCast{i}": "0" for i in range(1, 5)})
    quest.update({"SrcItemId": "0", "LimitTime": "0", "SuggestedPlayers": "1", "Type": "0"})
    quest.update(overrides)
    return quest


def make_data(**overrides):
    data = ZoneData(
        quests={},
        starters_creature={},
        finishers_creature={},
        starters_go={},
        finishers_go={},
        explore_triggers={},
        scripted_starts=set(),
        creature_templates={100: {"name": "Moonrage Whitescalp", "min_level": 9, "max_level": 10, "npc_flags": 0}},
        go_templates={},
        item_names={200: "Courier Bundle", 201: "Wolf Heart"},
        creature_spawns={100: [Point(500.0, 1600.0, 100.0), Point(520.0, 1610.0, 100.0)]},
        go_spawns={},
        item_sources={201: {"creature": [100], "gameobject": []}},
    )
    for key, value in overrides.items():
        setattr(data, key, value)
    return data


def test_cluster_points_groups_close_spawns():
    points = [Point(0, 0, 0), Point(10, 10, 0), Point(1000, 1000, 0)]
    hotspots = cluster_points(points)
    assert len(hotspots) == 2
    assert hotspots[0]["spawn_count"] == 2


def test_kill_objective_gets_hotspots():
    quest = make_quest(ReqCreatureOrGOId1="100", ReqCreatureOrGOCount1="5")
    objectives, reasons = classify_objectives(1, quest, make_data())
    assert objectives[0]["kind"] == "kill" and objectives[0]["count"] == 5
    assert objectives[0]["hotspots"] and not reasons


def test_kill_objective_without_spawns_is_flagged():
    quest = make_quest(ReqCreatureOrGOId1="100", ReqCreatureOrGOCount1="5")
    _, reasons = classify_objectives(1, quest, make_data(creature_spawns={}))
    assert any("no in-zone spawns" in r for r in reasons)


def test_courier_item_classifies_as_deliver():
    quest = make_quest(SrcItemId="200", ReqItemId1="200", ReqItemCount1="1")
    objectives, reasons = classify_objectives(1, quest, make_data())
    assert objectives[0]["kind"] == "deliver" and not reasons


def test_collect_without_static_spawns_is_flagged():
    quest = make_quest(ReqItemId1="201", ReqItemCount1="6")
    objectives, reasons = classify_objectives(1, quest, make_data(creature_spawns={}))
    assert objectives[0]["kind"] == "collect_drop"
    assert any("no in-zone static spawns" in r for r in reasons)


def test_spellcast_requirement_is_flagged():
    quest = make_quest(ReqSpellCast1="1234")
    _, reasons = classify_objectives(1, quest, make_data())
    assert any("spell cast" in r for r in reasons)


def test_shipped_silverpine_artifact_shape():
    document = json.loads(SILVERPINE_JSON.read_text())
    assert document["schema_version"] == 1
    assert document["zone"]["id"] == 130
    assert document["summary"]["quests"] >= 30
    fraction = document["summary"]["implementable"] / document["summary"]["quests"]
    assert fraction > 0.5
    for quest in document["quests"]:
        assert {"id", "title", "objectives", "classification", "start", "end"} <= set(quest)


def test_shipped_barrens_artifact_shape_and_loot_resolution():
    document = json.loads(BARRENS_JSON.read_text())
    assert document["schema_version"] == 1
    assert document["zone"] == {
        "id": 17,
        "name": "The Barrens",
        "map": 1,
        "bbox": {"min_x": -4300.0, "max_x": 1300.0, "min_y": -4700.0, "max_y": -900.0},
        "quest_level_band": [10, 60],
    }
    assert document["summary"]["quests"] >= 90
    fraction = document["summary"]["implementable"] / document["summary"]["quests"]
    assert fraction > 0.55

    quests = {quest["id"]: quest for quest in document["quests"]}
    fungal_spores = quests[848]
    assert fungal_spores["classification"]["implementable"]
    objective = fungal_spores["objectives"][0]
    assert objective["kind"] == "collect_object"
    # Loot template 2496 resolves through chest data1 to Laden Mushroom.
    assert 3640 in objective["source_gameobjects"]
    assert objective["hotspots"]

    thunder_bluff_ride = quests[6362]
    assert not thunder_bluff_ride["classification"]["implementable"]
    assert any("ends outside zone" in reason for reason in thunder_bluff_ride["classification"]["reasons"])

    for quest in document["quests"]:
        if not quest["classification"]["implementable"]:
            continue
        if quest["start"]:
            assert any(endpoint["spawns"] for endpoint in quest["start"])
        if quest["end"]:
            assert any(endpoint["spawns"] for endpoint in quest["end"])
        for objective in quest["objectives"]:
            if objective["kind"] in ("kill", "collect_drop", "collect_object", "interact_go"):
                assert objective["hotspots"], (quest["id"], objective["kind"])
