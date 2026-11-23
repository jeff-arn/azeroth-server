#!/usr/bin/env python3
"""Reference vertical slice of the leveling bot planner + skills.

See docs/BOT_LEVELING_DESIGN.md. This is the Python reference
implementation chosen 2026-11-23: it validates the greedy planner and
the skill contracts end-to-end over extracted zone data
(data/zones/silverpine.json) with a deterministic simulated world, and
serves as the behavioral spec/oracle for the eventual Rust port in
vmangos-rs (mangos-bots). It does NOT talk to a game server.

World model approximations (documented, deliberately coarse):
- Travel is euclidean at run speed; no pathing, terrain, or ambushes.
- Combat is a closed-form TTK/damage model per class with seeded RNG
  jitter; no rotations, adds, or patrols. GiveLevel-style runtime
  morphing means bots start competent for their level.
- Quest XP uses RewXP with the vanilla grey-quest falloff; mob XP uses
  the standard base formula; level thresholds come from the world DB
  `player_xp_for_level` table (embedded below, levels 1-30).
- Respawn availability is per-hotspot with a fixed respawn timer.
- Drop chance is a single tunable; the extractor does not yet capture
  per-item loot chances.

Every decision and skill outcome is appended to a structured event log
(JSONL) in the shape the bench design calls for: run id, bot, sim time,
event type, and decision inputs (top scored candidates on each plan).
"""

from __future__ import annotations

import argparse
import json
import logging
import math
import random
import sys
from dataclasses import dataclass, field
from pathlib import Path

LOG = logging.getLogger("bot_sim")

# player_xp_for_level.xp_for_next_level for levels 1..30 (world DB).
XP_FOR_NEXT = {
    1: 400,
    2: 900,
    3: 1400,
    4: 2100,
    5: 2800,
    6: 3600,
    7: 4500,
    8: 5400,
    9: 6500,
    10: 7600,
    11: 8800,
    12: 10100,
    13: 11400,
    14: 12900,
    15: 14400,
    16: 16000,
    17: 17700,
    18: 19400,
    19: 21300,
    20: 23200,
    21: 25200,
    22: 27300,
    23: 29400,
    24: 31700,
    25: 34000,
    26: 36400,
    27: 38900,
    28: 41400,
    29: 44300,
    30: 47400,
}

RUN_SPEED_YPS = 7.0


@dataclass
class Params:
    """Tunables the (future) hill climb moves. Defaults are starting points."""

    engage_min_health_pct: float = 0.65
    engage_min_mana_pct: float = 0.45
    drink_below_mana_pct: float = 0.35
    eat_below_health_pct: float = 0.55
    recover_rate_pct_per_s: float = 0.045
    quest_level_window_above: int = 2
    bags_towntrip_pct: float = 0.85
    abandon_after_deaths: int = 3
    drop_chance: float = 0.45
    respawn_seconds: float = 240.0
    overlap_bonus_per_quest: float = 0.08
    overlap_radius_yards: float = 300.0
    grind_when_no_quest: bool = True


@dataclass(frozen=True)
class ClassModel:
    name: str
    race_id: int
    dps_base: float
    dps_per_level: float
    hp_base: float
    hp_per_level: float
    mana_base: float
    mana_per_level: float
    damage_taken_factor: float  # mitigation/pet/kiting composite
    mana_per_kill_pct: float


CLASS_MODELS = {
    "warrior": ClassModel("warrior", 5, 9.0, 2.2, 90.0, 22.0, 0.0, 0.0, 0.75, 0.0),
    "hunter": ClassModel("hunter", 8, 10.0, 2.4, 80.0, 18.0, 60.0, 16.0, 0.55, 0.06),
    # Priest factor folds Power Word: Shield + heals into the composite;
    # the cost shows up as high mana burn per kill instead of damage.
    "priest": ClassModel("priest", 5, 8.0, 2.0, 70.0, 15.0, 80.0, 20.0, 0.50, 0.25),
}


def mob_hp(level: int) -> float:
    return 20.0 * level + 4.5 * (level**1.6)


def expected_damage_pct(model: ClassModel, bot_level: int, mob_level: int) -> float:
    """Mean fraction of max HP one kill costs (mirrors Runner.fight_one)."""
    diff = mob_level - bot_level
    dps = (model.dps_base + model.dps_per_level * bot_level) * max(0.55, 1.0 - 0.07 * max(diff, 0))
    ttk = mob_hp(mob_level) / max(dps, 1.0)
    mob_dps = 1.6 * mob_level * (1.0 + 0.18 * max(diff, 0)) * model.damage_taken_factor
    return (mob_dps * ttk) / (model.hp_base + model.hp_per_level * bot_level)


def mob_xp(bot_level: int, mob_level: int) -> int:
    """Vanilla-style mob XP: base by mob level, adjusted by difference."""
    if mob_level <= bot_level - 6:
        return 0
    base = 45.0 + 5.0 * mob_level
    diff = mob_level - bot_level
    if diff > 0:
        factor = 1.0 + 0.05 * diff
    else:
        factor = max(0.15, 1.0 - 0.12 * (-diff))
    return int(base * factor)


def quest_xp_scaled(rew_xp: int, quest_level: int, bot_level: int) -> int:
    """Grey-quest falloff: full within 5 levels, then -20%/level to 5%."""
    diff = bot_level - max(quest_level, 1)
    if diff <= 5:
        return rew_xp
    return max(1, int(rew_xp * max(0.05, 1.0 - 0.2 * (diff - 5))))


@dataclass
class QuestProgress:
    quest_id: int
    counts: list[int]  # per objective index
    deaths: int = 0


@dataclass
class Bot:
    model: ClassModel
    level: int
    xp: int = 0
    x: float = 0.0
    y: float = 0.0
    health_pct: float = 1.0
    mana_pct: float = 1.0
    money_copper: int = 5000
    bags_used: float = 0.0
    bags_capacity: float = 40.0
    time_s: float = 0.0
    log: dict[int, QuestProgress] = field(default_factory=dict)
    completed: set[int] = field(default_factory=set)
    blacklisted: set[int] = field(default_factory=set)
    deaths: int = 0
    kills: int = 0

    @property
    def pos(self) -> tuple[float, float]:
        return (self.x, self.y)

    def max_hp(self) -> float:
        return self.model.hp_base + self.model.hp_per_level * self.level

    def gain_xp(self, amount: int, events: list, source: str) -> None:
        self.xp += amount
        while self.level in XP_FOR_NEXT and self.xp >= XP_FOR_NEXT[self.level]:
            self.xp -= XP_FOR_NEXT[self.level]
            self.level += 1
            self.health_pct = 1.0
            self.mana_pct = 1.0
            events.append(("level_up", {"level": self.level, "source": source}))


def dist(a: tuple[float, float], b: tuple[float, float]) -> float:
    return math.hypot(a[0] - b[0], a[1] - b[1])


class Zone:
    """Read model over one extracted zone JSON document."""

    def __init__(self, document: dict):
        self.document = document
        self.quests: dict[int, dict] = {q["id"]: q for q in document["quests"]}
        self.npcs: dict[int, dict] = {n["npc_id"]: n for n in document["quest_npcs"]}
        self.hubs: list[dict] = document["hubs"]
        self.grind_spots: list[dict] = document.get("grind_spots", [])

    @staticmethod
    def first_spawn(endpoint_list: list[dict]) -> tuple[float, float] | None:
        for endpoint in endpoint_list:
            if endpoint.get("spawns"):
                spawn = endpoint["spawns"][0]
                return (spawn["x"], spawn["y"])
        return None

    def objective_point(
        self, quest: dict, objective: dict, from_pos: tuple[float, float]
    ) -> tuple[float, float] | None:
        kind = objective["kind"]
        if kind in ("kill", "collect_drop", "collect_object", "interact_go"):
            hotspots = objective.get("hotspots") or []
            if not hotspots:
                return None
            best = min(hotspots, key=lambda h: dist(from_pos, (h["x"], h["y"])))
            return (best["x"], best["y"])
        if kind in ("deliver", "talk"):
            return self.first_spawn(quest["end"])
        return None

    def hub_points(self) -> list[tuple[float, float]]:
        return [(h["center"]["x"], h["center"]["y"]) for h in self.hubs]


class Runner:
    """Executes skills for one bot and records structured events."""

    def __init__(self, zone: Zone, bot: Bot, params: Params, rng: random.Random):
        self.zone = zone
        self.bot = bot
        self.params = params
        self.rng = rng
        self.events: list[dict] = []
        # hotspot key -> list of sim times when a spawn returns
        self.respawn_queue: dict[tuple[int, int], list[float]] = {}
        self.current_quest: int | None = None

    # -- plumbing ---------------------------------------------------------

    def emit(self, event_type: str, **details) -> None:
        self.events.append(
            {
                "t": round(self.bot.time_s, 1),
                "bot": self.bot.model.name,
                "event": event_type,
                "level": self.bot.level,
                **details,
            }
        )

    def advance(self, seconds: float) -> None:
        self.bot.time_s += max(0.0, seconds)

    # -- skills -----------------------------------------------------------

    def travel_to(self, point: tuple[float, float], why: str) -> None:
        seconds = dist(self.bot.pos, point) / RUN_SPEED_YPS
        self.advance(seconds)
        self.bot.x, self.bot.y = point
        self.emit("travel", why=why, seconds=round(seconds, 1), to=[round(point[0], 1), round(point[1], 1)])

    def recover(self, why: str) -> None:
        bot, params = self.bot, self.params
        need = 0.0
        if bot.health_pct < 1.0:
            need = max(need, (1.0 - bot.health_pct) / params.recover_rate_pct_per_s)
        if bot.model.mana_per_level > 0 and bot.mana_pct < 1.0:
            need = max(need, (1.0 - bot.mana_pct) / params.recover_rate_pct_per_s)
        if need > 0:
            self.advance(need)
            bot.health_pct = 1.0
            bot.mana_pct = 1.0
            self.emit("recover", why=why, seconds=round(need, 1))

    def needs_recover_before_pull(self) -> bool:
        bot, params = self.bot, self.params
        if bot.health_pct < params.engage_min_health_pct:
            return True
        return bot.model.mana_per_level > 0 and bot.mana_pct < params.engage_min_mana_pct

    def fight_one(self, mob_level: int) -> bool:
        """One kill attempt. Returns True if the bot survived and killed."""
        bot = self.bot
        diff = mob_level - bot.level
        dps = (bot.model.dps_base + bot.model.dps_per_level * bot.level) * max(0.55, 1.0 - 0.07 * max(diff, 0))
        ttk = mob_hp(mob_level) / max(dps, 1.0)
        ttk *= self.rng.uniform(0.9, 1.1)
        mob_dps = 1.6 * mob_level * (1.0 + 0.18 * max(diff, 0)) * bot.model.damage_taken_factor
        damage_pct = (mob_dps * ttk) / bot.max_hp() * self.rng.uniform(0.85, 1.15)
        self.advance(ttk)
        bot.mana_pct = max(0.0, bot.mana_pct - bot.model.mana_per_kill_pct)
        bot.health_pct -= damage_pct
        if bot.health_pct <= 0:
            self.die(mob_level)
            return False
        bot.kills += 1
        bot.bags_used = min(bot.bags_capacity, bot.bags_used + 0.7)
        level_events: list = []
        bot.gain_xp(mob_xp(bot.level, mob_level), level_events, "kill")
        for event_type, details in level_events:
            self.emit(event_type, **details)
        return True

    def die(self, mob_level: int) -> None:
        bot = self.bot
        bot.deaths += 1
        if self.current_quest is not None and self.current_quest in bot.log:
            bot.log[self.current_quest].deaths += 1
        self.emit("death", mob_level=mob_level, quest=self.current_quest)
        # Corpse run from the nearest hub (graveyard stand-in).
        hubs = self.zone.hub_points()
        hub = min(hubs, key=lambda h: dist(bot.pos, h)) if hubs else bot.pos
        self.advance(120.0 + dist(bot.pos, hub) / RUN_SPEED_YPS)
        bot.x, bot.y = hub
        bot.health_pct = 1.0
        bot.mana_pct = 1.0
        self.emit("corpse_run", to=[round(hub[0], 1), round(hub[1], 1)])

    def hotspot_available(self, hotspot: dict) -> int:
        key = (int(hotspot["x"]), int(hotspot["y"]))
        queue = [t for t in self.respawn_queue.get(key, []) if t > self.bot.time_s]
        self.respawn_queue[key] = queue
        return max(0, int(hotspot["spawn_count"]) - len(queue))

    def use_spawn(self, hotspot: dict) -> None:
        key = (int(hotspot["x"]), int(hotspot["y"]))
        self.respawn_queue.setdefault(key, []).append(self.bot.time_s + self.params.respawn_seconds)

    def pick_hotspot(self, hotspots: list[dict]) -> dict | None:
        available = [h for h in hotspots if self.hotspot_available(h) > 0]
        if not available:
            return None
        return min(available, key=lambda h: dist(self.bot.pos, (h["x"], h["y"])))

    def do_kill_like(self, quest: dict, objective: dict, progress: QuestProgress, index: int) -> bool:
        """KillObjective / CollectObjective(drops): fight until count met.

        Returns False when the bot died (caller replans).
        """
        bot = self.bot
        is_drop = objective["kind"] == "collect_drop"
        mob_level = self.objective_mob_level(objective, quest)
        while progress.counts[index] < objective["count"]:
            hotspot = self.pick_hotspot(objective.get("hotspots") or [])
            if hotspot is None:
                # Everything is on respawn: wait out the shortest timer.
                waits = [
                    min(q) - bot.time_s
                    for q in (
                        self.respawn_queue.get((int(h["x"]), int(h["y"])), []) for h in objective.get("hotspots", [])
                    )
                    if q
                ]
                wait = min(waits) if waits else self.params.respawn_seconds
                self.emit("wait_respawn", quest=quest["id"], seconds=round(wait, 1))
                self.advance(wait)
                continue
            self.travel_to((hotspot["x"], hotspot["y"]), f"quest {quest['id']} hotspot")
            if self.needs_recover_before_pull():
                self.recover("pre-pull")
            self.use_spawn(hotspot)
            if not self.fight_one(mob_level):
                return False
            if not is_drop or self.rng.random() < self.params.drop_chance:
                progress.counts[index] += 1
                self.advance(2.0)  # loot
            if bot.mana_pct < self.params.drink_below_mana_pct or bot.health_pct < self.params.eat_below_health_pct:
                self.recover("between pulls")
        self.emit("objective_done", quest=quest["id"], kind=objective["kind"], count=objective["count"])
        return True

    def do_collect_object(self, quest: dict, objective: dict, progress: QuestProgress, index: int) -> bool:
        while progress.counts[index] < objective["count"]:
            hotspot = self.pick_hotspot(objective.get("hotspots") or [])
            if hotspot is None:
                self.advance(self.params.respawn_seconds / 2)
                continue
            self.travel_to((hotspot["x"], hotspot["y"]), f"quest {quest['id']} object")
            self.use_spawn(hotspot)
            self.advance(3.0)  # interact channel
            progress.counts[index] += 1
        self.emit("objective_done", quest=quest["id"], kind=objective["kind"], count=objective["count"])
        return True

    def do_talk_like(self, quest: dict) -> bool:
        point = self.zone.first_spawn(quest["end"])
        if point is None:
            return False
        self.travel_to(point, f"quest {quest['id']} turn-in talk")
        self.advance(2.0)
        return True

    def objective_mob_level(self, objective: dict, quest: dict) -> int:
        if objective.get("creature_level") and objective["creature_level"][1]:
            return int(objective["creature_level"][1])
        # Drop-source creature levels are not in the zone JSON yet; quest
        # level is the standard stand-in (objectives mobs sit at it ±1).
        return int(quest.get("quest_level") or self.bot.level)

    def work_quest(self, quest: dict) -> str:
        """Execute all incomplete objectives of a held quest.

        Returns "complete", "died", or "stuck".
        """
        bot = self.bot
        progress = bot.log[quest["id"]]
        self.current_quest = quest["id"]
        try:
            for index, objective in enumerate(quest["objectives"]):
                if progress.counts[index] >= objective.get(
                    "count", 1 if objective["kind"] in ("talk", "deliver") else 0
                ):
                    continue
                kind = objective["kind"]
                if kind in ("kill", "collect_drop"):
                    if not self.do_kill_like(quest, objective, progress, index):
                        return "died"
                elif kind in ("collect_object", "interact_go"):
                    self.do_collect_object(quest, objective, progress, index)
                elif kind in ("deliver", "talk"):
                    if not self.do_talk_like(quest):
                        return "stuck"
                    progress.counts[index] = 1
                else:
                    return "stuck"
            return "complete"
        finally:
            self.current_quest = None

    def turn_in(self, quest: dict) -> None:
        bot = self.bot
        point = self.zone.first_spawn(quest["end"])
        if point is not None:
            self.travel_to(point, f"turn in {quest['id']}")
        self.advance(2.0)
        gained = quest_xp_scaled(quest["rewards"]["xp_column"], quest["quest_level"], bot.level)
        bot.money_copper += max(0, quest["rewards"]["money_copper"])
        del bot.log[quest["id"]]
        bot.completed.add(quest["id"])
        level_events: list = []
        bot.gain_xp(gained, level_events, f"quest {quest['id']}")
        self.emit("quest_turned_in", quest=quest["id"], xp=gained)
        for event_type, details in level_events:
            self.emit(event_type, **details)

    def accept_quest(self, quest: dict) -> None:
        point = self.zone.first_spawn(quest["start"])
        if point is not None and dist(self.bot.pos, point) > 5:
            self.travel_to(point, f"accept {quest['id']}")
        self.advance(2.0)
        counts = [0] * len(quest["objectives"])
        self.bot.log[quest["id"]] = QuestProgress(quest_id=quest["id"], counts=counts)
        self.emit("quest_accepted", quest=quest["id"], title=quest["title"])

    def town_trip(self) -> None:
        hubs = self.zone.hub_points()
        if not hubs:
            return
        hub = min(hubs, key=lambda h: dist(self.bot.pos, h))
        self.travel_to(hub, "town trip")
        sold = int(self.bot.bags_used * 35)
        self.bot.money_copper += sold
        self.bot.bags_used = 0.0
        self.advance(20.0)
        self.recover("town trip")
        self.emit("town_trip", sold_copper=sold, money_copper=self.bot.money_copper)

    def grind_until(self, target_level: int, max_seconds: float, mob_level_cap: int) -> int:
        """GrindFiller: kill at the nearest level-appropriate grind spot.

        Returns deaths suffered. Aborts early after 2 deaths so the
        planner can step the mob level down instead of corpse-looping.
        """
        bot = self.bot
        spots = [
            s
            for s in self.zone.grind_spots
            if s["level_range"][0] <= mob_level_cap and s["level_range"][1] >= bot.level - 5
        ]
        if not spots:
            self.advance(600.0)
            self.emit("grind_no_spot", mob_level_cap=mob_level_cap)
            return 0
        spot = min(spots, key=lambda s: dist(bot.pos, (s["center"]["x"], s["center"]["y"])))
        self.travel_to((spot["center"]["x"], spot["center"]["y"]), "grind spot")
        mob_level = max(1, min(spot["level_range"][1], mob_level_cap))
        self.emit("grind_start", mob_level=mob_level, spawns=spot["spawn_count"])
        deadline = bot.time_s + max_seconds
        deaths = 0
        while bot.level < target_level and bot.time_s < deadline:
            if self.needs_recover_before_pull():
                self.recover("grind")
            if not self.fight_one(mob_level):
                deaths += 1
                if deaths >= 2:
                    self.emit("grind_aborted", mob_level=mob_level, deaths=deaths)
                    return deaths
                self.travel_to((spot["center"]["x"], spot["center"]["y"]), "grind spot (return)")
                continue
            if bot.mana_pct < self.params.drink_below_mana_pct or bot.health_pct < self.params.eat_below_health_pct:
                self.recover("grind")
        return deaths


# -- planner -----------------------------------------------------------------


def quest_available(quest: dict, bot: Bot, params: Params) -> bool:
    if not quest["classification"]["implementable"]:
        return False
    if quest["id"] in bot.completed or quest["id"] in bot.log or quest["id"] in bot.blacklisted:
        return False
    if bot.level < quest["min_level"]:
        return False
    if quest["quest_level"] > bot.level + params.quest_level_window_above:
        return False
    prev = quest["prev_quest"]
    if prev > 0 and prev not in bot.completed:
        return False
    if prev < 0 and (-prev) not in bot.completed and (-prev) not in bot.log:
        return False
    races = quest["required_races_mask"]
    if races and not (races & (1 << (bot.model.race_id - 1))):
        return False
    return True


def quest_finished(quest: dict, bot: Bot) -> bool:
    progress = bot.log.get(quest["id"])
    if progress is None:
        return False
    for index, objective in enumerate(quest["objectives"]):
        needed = objective.get("count", 1)
        if progress.counts[index] < needed:
            return False
    return True


def estimate_quest(
    quest: dict, bot: Bot, zone: Zone, params: Params
) -> tuple[float, float, tuple[float, float] | None, float]:
    """(score xp/min, estimated xp, work point, survival 0..1) for one quest.

    Survival is the planner's risk input from the design: 1 minus the
    worst objective's expected HP cost per kill. Near-zero survival
    means a single pull is expected to kill the bot.
    """
    xp = quest_xp_scaled(quest["rewards"]["xp_column"], quest["quest_level"], bot.level)
    work_s = 0.0
    survival = 1.0
    work_point: tuple[float, float] | None = None
    for objective in quest["objectives"]:
        point = zone.objective_point(quest, objective, bot.pos)
        if work_point is None:
            work_point = point
        kind = objective["kind"]
        if kind == "kill":
            mob_level = (objective.get("creature_level") or [0, quest["quest_level"]])[1] or quest["quest_level"]
            xp += objective["count"] * mob_xp(bot.level, int(mob_level))
            work_s += objective["count"] * 22.0
            survival = min(survival, 1.0 - expected_damage_pct(bot.model, bot.level, int(mob_level)))
        elif kind == "collect_drop":
            kills = objective["count"] / max(params.drop_chance, 0.05)
            xp += kills * mob_xp(bot.level, quest["quest_level"])
            work_s += kills * 22.0
            survival = min(survival, 1.0 - expected_damage_pct(bot.model, bot.level, quest["quest_level"]))
        elif kind in ("collect_object", "interact_go"):
            work_s += objective["count"] * 12.0
        elif kind in ("deliver", "talk"):
            work_s += 5.0
    travel_s = dist(bot.pos, work_point) / RUN_SPEED_YPS if work_point else 0.0
    end_point = zone.first_spawn(quest["end"])
    if work_point and end_point:
        travel_s += 0.5 * dist(work_point, end_point) / RUN_SPEED_YPS
    minutes = max(0.25, (travel_s + work_s) / 60.0)
    survival = max(0.0, min(1.0, survival))
    return (xp / minutes) * (0.15 + 0.85 * survival), xp, work_point, survival


def run_bot(
    zone: Zone, class_name: str, start_level: int, target_level: int, max_hours: float, params: Params, seed: int
) -> tuple[Bot, list[dict]]:
    bot = Bot(model=CLASS_MODELS[class_name], level=start_level)
    hubs = zone.hub_points()
    if hubs:
        bot.x, bot.y = hubs[0]
    runner = Runner(zone, bot, params, random.Random(seed))
    runner.emit("run_start", start_level=start_level, target_level=target_level)
    deadline = max_hours * 3600.0
    idle_cycles = 0
    grind_cap_offset = 1  # grind mobs at bot level + offset; steps down on deaths
    grind_failures_at_floor = 0

    while bot.level < target_level and bot.time_s < deadline:
        # 1. Turn in anything finished when idle pressure says so.
        finished = [zone.quests[qid] for qid in bot.log if quest_finished(zone.quests[qid], bot)]
        bags_pct = bot.bags_used / bot.bags_capacity
        if bags_pct >= params.bags_towntrip_pct:
            for quest in finished:
                runner.turn_in(quest)
            runner.town_trip()
            idle_cycles = 0
            continue
        # 2. Score held incomplete quests and available new quests together.
        held = [zone.quests[qid] for qid in bot.log if not quest_finished(zone.quests[qid], bot)]
        available = [q for q in zone.quests.values() if quest_available(q, bot, params)]
        scored: list[tuple[float, dict, bool]] = []
        points = {}
        for quest, is_held in [(q, True) for q in held] + [(q, False) for q in available]:
            score, _, point, survival = estimate_quest(quest, bot, zone, params)
            points[quest["id"]] = point
            if survival < 0.25:
                # A single pull is expected to (nearly) kill the bot:
                # defer until it levels. Talk/deliver quests keep
                # survival 1.0 and are never deferred by this rule.
                continue
            scored.append((score, quest, is_held))
        adjusted: list[tuple[float, dict, bool]] = []
        for score, quest, is_held in scored:
            point = points[quest["id"]]
            neighbors = 0
            if point:
                neighbors = sum(
                    1
                    for other_id, other in points.items()
                    if other_id != quest["id"] and other and dist(point, other) <= params.overlap_radius_yards
                )
            adjusted.append((score * (1.0 + params.overlap_bonus_per_quest * neighbors), quest, is_held))
        adjusted.sort(key=lambda item: item[0], reverse=True)

        if not adjusted:
            if finished:
                for quest in finished:
                    runner.turn_in(quest)
                idle_cycles = 0
                continue
            if params.grind_when_no_quest:
                level_before = bot.level
                deaths = runner.grind_until(bot.level + 1, 1800.0, bot.level + grind_cap_offset)
                if bot.level > level_before:
                    grind_cap_offset = 1
                    grind_failures_at_floor = 0
                elif deaths:
                    if grind_cap_offset > -4:
                        grind_cap_offset -= 1
                        runner.emit("grind_step_down", new_cap_offset=grind_cap_offset)
                    else:
                        grind_failures_at_floor += 1
                        if grind_failures_at_floor >= 3:
                            runner.emit("run_stalled", reason="grinding lethal even at level-4 mobs")
                            break
                idle_cycles = 0
                continue
            idle_cycles += 1
            if idle_cycles > 3:
                runner.emit("run_stalled", reason="no candidates and grinding disabled")
                break
            runner.advance(60.0)
            continue

        top_score, chosen, is_held = adjusted[0]
        runner.emit(
            "plan",
            chosen=chosen["id"],
            held=is_held,
            score=round(top_score, 2),
            candidates=[[q["id"], round(s, 2)] for s, q, _ in adjusted[:3]],
        )
        idle_cycles = 0
        # Opportunistic hub work: if heading to accept/work near a hub and
        # we hold finished quests, turn them in en route (cheap heuristic:
        # always clear finished quests before accepting a brand-new one).
        if not is_held and finished:
            for quest in finished:
                runner.turn_in(quest)
        if not is_held:
            if len(bot.log) >= 20:  # vanilla quest log cap
                for quest in finished:
                    runner.turn_in(quest)
                if len(bot.log) >= 20:
                    runner.emit("run_stalled", reason="quest log full with nothing finished")
                    break
                continue
            runner.accept_quest(chosen)
        outcome = runner.work_quest(chosen)
        progress = bot.log.get(chosen["id"])
        if outcome == "complete":
            runner.turn_in(chosen)
        elif outcome == "stuck":
            bot.blacklisted.add(chosen["id"])
            bot.log.pop(chosen["id"], None)
            runner.emit("quest_abandoned", quest=chosen["id"], reason="stuck")
        elif outcome == "died" and progress is not None and progress.deaths >= params.abandon_after_deaths:
            bot.blacklisted.add(chosen["id"])
            bot.log.pop(chosen["id"], None)
            runner.emit("quest_abandoned", quest=chosen["id"], reason=f"{progress.deaths} deaths")

    runner.emit(
        "run_end",
        level=bot.level,
        quests_completed=len(bot.completed),
        deaths=bot.deaths,
        kills=bot.kills,
        sim_hours=round(bot.time_s / 3600.0, 2),
        money_copper=bot.money_copper,
    )
    return bot, runner.events


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--zone", type=Path, default=Path("data/zones/silverpine.json"))
    parser.add_argument("--classes", default="warrior,hunter,priest")
    parser.add_argument("--start-level", type=int, default=11)
    parser.add_argument("--target-level", type=int, default=20)
    parser.add_argument("--max-hours", type=float, default=16.0)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--out-dir", type=Path, default=Path("/tmp/bot_sim"))
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=logging.WARNING if args.quiet else logging.INFO, format="%(levelname)s %(name)s: %(message)s"
    )
    zone = Zone(json.loads(args.zone.read_text()))
    params = Params()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    summaries = []
    for class_name in [c.strip() for c in args.classes.split(",") if c.strip()]:
        if class_name not in CLASS_MODELS:
            raise SystemExit(f"unknown class {class_name!r} (have: {sorted(CLASS_MODELS)})")
        bot, events = run_bot(zone, class_name, args.start_level, args.target_level, args.max_hours, params, args.seed)
        events_path = args.out_dir / f"events-{class_name}.jsonl"
        events_path.write_text("".join(json.dumps(event) + "\n" for event in events))
        summary = {
            "class": class_name,
            "final_level": bot.level,
            "quests_completed": len(bot.completed),
            "deaths": bot.deaths,
            "kills": bot.kills,
            "sim_hours": round(bot.time_s / 3600.0, 2),
            "money_copper": bot.money_copper,
            "blacklisted": sorted(bot.blacklisted),
            "events_file": str(events_path),
        }
        summaries.append(summary)
        LOG.info(
            "%s: level %d, %d quests, %d deaths, %d kills, %.2f sim-hours",
            class_name,
            bot.level,
            len(bot.completed),
            bot.deaths,
            bot.kills,
            bot.time_s / 3600.0,
        )
    (args.out_dir / "summary.json").write_text(json.dumps(summaries, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
