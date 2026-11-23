"""Tests for scripts/bot_sim.py — planner/skills reference slice."""

import json
from pathlib import Path

import pytest
from bot_sim import (
    CLASS_MODELS,
    Bot,
    Params,
    Zone,
    expected_damage_pct,
    mob_xp,
    quest_available,
    quest_xp_scaled,
    run_bot,
)

SILVERPINE = Path(__file__).resolve().parents[2] / "data" / "zones" / "silverpine.json"


def make_bot(class_name="warrior", level=11):
    return Bot(model=CLASS_MODELS[class_name], level=level)


def make_quest(**overrides):
    quest = {
        "id": 1,
        "title": "Test",
        "quest_level": 11,
        "min_level": 9,
        "prev_quest": 0,
        "required_races_mask": 0,
        "classification": {"implementable": True, "reasons": []},
    }
    quest.update(overrides)
    return quest


def test_xp_math():
    assert mob_xp(11, 11) == 100  # 45 + 5*11
    assert mob_xp(20, 13) == 0  # grey
    assert quest_xp_scaled(1000, 10, 11) == 1000
    assert quest_xp_scaled(1000, 10, 20) == 50  # floor 5%


def test_availability_respects_chain_and_level():
    bot = make_bot()
    params = Params()
    assert quest_available(make_quest(), bot, params)
    assert not quest_available(make_quest(prev_quest=99), bot, params)
    bot.completed.add(99)
    assert quest_available(make_quest(prev_quest=99), bot, params)
    assert not quest_available(make_quest(quest_level=20), bot, params)  # window +2
    assert not quest_available(make_quest(min_level=15), bot, params)


def test_availability_skips_unimplementable_and_blacklisted():
    bot = make_bot()
    params = Params()
    quest = make_quest(classification={"implementable": False, "reasons": ["elite"]})
    assert not quest_available(quest, bot, params)
    bot.blacklisted.add(1)
    assert not quest_available(make_quest(), bot, params)


def test_survival_model_orders_classes():
    # At equal level the priest (shield/heal composite) survives pulls
    # that are riskier for the warrior; +3 mobs are dangerous for all.
    for name in ("warrior", "hunter", "priest"):
        even = expected_damage_pct(CLASS_MODELS[name], 11, 11)
        hard = expected_damage_pct(CLASS_MODELS[name], 11, 14)
        assert even < 0.75, name
        assert hard > even, name


@pytest.mark.integration
def test_full_run_reaches_target_level():
    zone = Zone(json.loads(SILVERPINE.read_text()))
    bot, events = run_bot(zone, "priest", 11, 14, 8.0, Params(), seed=7)
    assert bot.level >= 14
    assert len(bot.completed) >= 5
    assert bot.deaths < 25
    assert events[-1]["event"] == "run_end"
