#!/usr/bin/env python3
"""Spell inventory generator for the level-60 all-class spell campaign.

Reads the bench world DB effective `spell_template` rows (max build <=
supported per entry, the same gate as
`mangos-world-data/src/spell_template.rs`) plus `spell_chain` and
`spell_disabled`, and emits `spell_campaign_matrix.json` next to this
script. The harness (`spell_campaign.py`) never queries `spell_template`
itself; this file is the only place matrix rows are derived.

Two modes:

* `--pilot` (default): emit only the hand-pinned pilot rows the
  architecture gates on (Warrior 5242/6546, Mage 168/133, Druid
  5187/5487/768, plus classification rows 6795 Growl and 20484 Rebirth).
  Pilot spell IDs are pinned evidence probes, not necessarily last
  ranks; generated rows state each cast ID's rank and complete chain.
  G1 (SkillRaceClassInfo join) and G2 (51-point talent builds) are
  recorded as pending-for-fan-out caveats; the pilot does not need them.
* `--g1`: replay the exact level-60 train-mode
  `IsSpellFitByClassAndRace` gate over the derived trainable set
  and write `spell_campaign_g1.json`.
* `--talent-builds`: enumerate the 26 checked-in 51-point builds
  through Talent.dbc RankID/DependsOn closure, audit fork legality,
  and write `spell_campaign_talent_builds.json`. Exits 2 if any
  class has no legal selected build.
* `--self-test`: offline invariant checks, no DB needed. Prints
  SPELL-MATRIX-GEN-SELFTEST-PASS.

`--full` (whole-matrix fan-out) exits 2 until G1/G2 are closed here, per
the architecture: fan-out is forbidden before the generator proves the
G1 narrowing delta and the per-class 51-point build expansion.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import struct
import subprocess
import sys

SUPPORTED_BUILD = 5875
SCRIPT_DIR = pathlib.Path(__file__).parent
MATRIX_PATH = SCRIPT_DIR / "spell_campaign_matrix.json"
TALENT_BUILDS_PATH = SCRIPT_DIR / "spell_campaign_talent_builds.json"
G1_AUDIT_PATH = SCRIPT_DIR / "spell_campaign_g1.json"
TALENT_BUILDS_RS = (
    SCRIPT_DIR.parent / "crates" / "mangos-bots" / "src" / "talent_builds.rs")
CLASS_IDS = {
    "Warrior": 1, "Paladin": 2, "Hunter": 3, "Rogue": 4,
    "Priest": 5, "Shaman": 7, "Mage": 8, "Warlock": 9, "Druid": 11,
}
PREFERRED_TALENT_ROLES = ("MeleeDps", "RangeDps", "Tank", "Healer")
CLASS_FAMILIES = {
    "Warrior": 4, "Paladin": 10, "Hunter": 9, "Rogue": 8,
    "Priest": 6, "Shaman": 11, "Mage": 3, "Warlock": 5, "Druid": 7,
}
ABILITY_SKILL_NONTRAINABLE = 0x100

# Buckets and expected outcomes below were derived from
# SPELL_MATRIX.md S1-S12 and verified against the oracle bodies named
# there. Rank metadata is deliberately absent here: the generator
# derives it from spell_chain and asserts each row still resolves in
# the bench DB before emitting it.
PILOT_ROWS: dict[int, dict] = {
    5242: {
        "class": "Warrior", "bucket": "self",
        "expected": {"kind": "aura_on_caster", "aura": 5242},
    },
    6546: {
        "class": "Warrior", "bucket": "hostile_dummy",
        "expected": {"kind": "health_delta_target", "aura": 6546},
    },
    168: {
        "class": "Mage", "bucket": "self",
        "expected": {"kind": "aura_on_caster", "aura": 168},
    },
    133: {
        "class": "Mage", "bucket": "hostile_dummy",
        "expected": {"kind": "damage_log", "opcode": 592},
    },
    5187: {
        # Rank 3, not rank 1: a rank-1 heal (oracle band 40-54) sits
        # inside the bench regen-tick noise (+34/1.5s, measured), so a
        # single window cannot separate it from a phase-aligned tick.
        # Rank 3 separates completely. This is a pinned pilot probe,
        # not the chain's last rank; generated metadata says so.
        "class": "Druid", "bucket": "friendly",
        "expected": {"kind": "heal_delta_target",
                     "heal_band": [204, 254],
                     "observed_jump_band": [170, 288]},
    },
    5487: {
        "class": "Druid", "bucket": "form",
        "expected": {"kind": "form_change", "aura": 5487},
    },
    768: {
        "class": "Druid", "bucket": "form",
        "expected": {"kind": "form_change", "aura": 768},
    },
    6795: {
        "class": "Druid", "bucket": "form",
        "expected": {"kind": "expect_cast_failure",
                     "note": "Growl in caster form; stance gate or failure"},
        "campaign": "classify_only_wrong_form",
    },
    20484: {
        "class": "Druid", "bucket": "corpse",
        "expected": {"kind": "resurrect_request"},
        "campaign": "classify_only_corpse",
    },
    8921: {
        "class": "Druid", "bucket": "hostile_dummy",
        "expected": {"kind": "health_delta_target", "aura": 8921},
    },
}

VALID_BUCKETS = {
    "self", "friendly", "hostile_dummy", "pet", "corpse",
    "ground_area", "item_reagent_ammo", "party_duel", "form_totem",
    "form", "passive",
}
VALID_EXPECTED = {
    "aura_on_caster", "aura_on_target", "health_delta_target",
    "heal_delta_target", "damage_log", "object_created", "pet_created",
    "form_change", "item_consumed", "learned_spell", "resurrect_request",
    "none_marker", "expect_cast_failure",
}

sys.path.insert(0, str(SCRIPT_DIR))


def _bench_password() -> str:
    import re

    env = pathlib.Path("vmangos-rs/.env.test").read_text()
    return re.search(r"DEV_MYSQL_PASSWORD=(\S+)", env).group(1).strip("'\"")


SPELL_ATTR_PASSIVE = 0x00000040


def _query(stmt: str) -> str:
    import os

    os.environ.setdefault("MYSQL_HOST", "127.0.0.1")
    r = subprocess.run(
        ["mysql", "-h", "127.0.0.1", "-u", "mangos",
         f"-p{_bench_password()}", "mangos", "-N", "-e", stmt],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"bench query failed: {r.stderr.strip()}")
    return r.stdout.strip()


def _query_rows(stmt: str) -> list[list[str]]:
    out = _query(stmt)
    if not out:
        return []
    return [line.split("\t") for line in out.splitlines()]


def effective_row(spell_id: int) -> dict:
    """One effective spell_template row (max build <= supported)."""
    fields = [
        ("entry", int), ("build", int), ("name", str),
        ("attributes", int), ("power_type", int), ("mana_cost", int),
        ("range_index", int), ("cast_time_index", int),
        ("duration_index", int), ("speed", float),
        ("stances", int), ("stances_not", int),
        ("totem1", int), ("totem2", int),
        ("reagent1", int), ("reagent2", int), ("reagent3", int),
        ("reagent4", int), ("reagent5", int), ("reagent6", int),
        ("reagent7", int), ("reagent8", int),
        ("equipped_item_class", int),
        ("effect1", int), ("effect2", int), ("effect3", int),
        ("aura1", int), ("aura2", int), ("aura3", int),
        ("targ_a1", int), ("targ_a2", int), ("targ_a3", int),
        ("targ_b1", int), ("targ_b2", int), ("targ_b3", int),
        ("script_name", str),
    ]
    out = _query(
        "SELECT entry,build,name,attributes,powerType,manaCost,"
        "rangeIndex,castingTimeIndex,durationIndex,speed,stances,"
        "stancesNot,totem1,totem2,reagent1,reagent2,reagent3,"
        "reagent4,reagent5,reagent6,reagent7,reagent8,"
        "equippedItemClass,effect1,effect2,effect3,"
        "effectApplyAuraName1,effectApplyAuraName2,"
        "effectApplyAuraName3,effectImplicitTargetA1,"
        "effectImplicitTargetA2,effectImplicitTargetA3,"
        "effectImplicitTargetB1,effectImplicitTargetB2,"
        "effectImplicitTargetB3,script_name "
        f"FROM spell_template t1 WHERE entry={spell_id} AND "
        f"build=(SELECT max(build) FROM spell_template t2 "
        f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})"
    )
    if not out:
        raise AssertionError(
            f"spell {spell_id}: no effective row at build <={SUPPORTED_BUILD}"
        )
    cols = out.split("\t")
    if len(cols) == len(fields) - 1:
        # _query strips the trailing tab of an empty script_name.
        cols.append("")
    if len(cols) != len(fields):
        raise AssertionError(
            f"spell {spell_id}: effective row has {len(cols)} fields, "
            f"want {len(fields)}")
    row = {key: conv(value) for (key, conv), value in zip(fields, cols)}
    # Keep the old short keys for any local probe importing this helper.
    row.update({
        "e1": row["effect1"], "e2": row["effect2"],
        "e3": row["effect3"], "targA1": row["targ_a1"],
        "targB1": row["targ_b1"], "durationIndex": row["duration_index"],
        "rangeIndex": row["range_index"],
    })
    return row


def derive_chain_metadata(chain_rows: list[dict], spell_id: int) -> dict:
    """Resolve and validate one spell_chain in S7 rank order."""
    if not chain_rows:
        return {
            "chain_first": spell_id, "rank": 1, "all_rank_ids": [spell_id],
            "last_rank_id": spell_id, "rank_count": 1,
            "is_last_rank": True,
        }
    rows = sorted(chain_rows, key=lambda r: int(r["rank"]))
    first = int(rows[0]["first_spell"])
    all_rank_ids = [int(r["spell_id"]) for r in rows]
    if all_rank_ids.count(spell_id) != 1:
        raise AssertionError(
            f"spell {spell_id}: chain {first} does not contain it once")
    for index, chain_row in enumerate(rows, start=1):
        if int(chain_row["first_spell"]) != first:
            raise AssertionError(f"spell chain {first}: mixed first_spell")
        if int(chain_row["rank"]) != index:
            raise AssertionError(f"spell chain {first}: rank gap at {index}")
        expected_prev = 0 if index == 1 else all_rank_ids[index - 2]
        if int(chain_row["prev_spell"]) != expected_prev:
            raise AssertionError(
                f"spell chain {first}: prev_spell at rank {index} is "
                f"{chain_row['prev_spell']}, want {expected_prev}")
    rank = all_rank_ids.index(spell_id) + 1
    return {
        "chain_first": first, "rank": rank, "all_rank_ids": all_rank_ids,
        "last_rank_id": all_rank_ids[-1], "rank_count": len(all_rank_ids),
        "is_last_rank": spell_id == all_rank_ids[-1],
    }


def chain_metadata(spell_id: int) -> dict:
    selected = _query_rows(
        "SELECT spell_id,prev_spell,first_spell,`rank`,req_spell "
        f"FROM spell_chain WHERE spell_id={spell_id} "
        f"AND build_min <= {SUPPORTED_BUILD} "
        f"AND build_max >= {SUPPORTED_BUILD}")
    if not selected:
        return derive_chain_metadata([], spell_id)
    if len(selected) != 1:
        raise AssertionError(
            f"spell {spell_id}: {len(selected)} active spell_chain rows")
    first = int(selected[0][2])
    raw = _query_rows(
        "SELECT spell_id,prev_spell,first_spell,`rank`,req_spell "
        f"FROM spell_chain WHERE first_spell={first} "
        f"AND build_min <= {SUPPORTED_BUILD} "
        f"AND build_max >= {SUPPORTED_BUILD}")
    keys = ("spell_id", "prev_spell", "first_spell", "rank", "req_spell")
    rows = [{key: int(value) for key, value in zip(keys, row)}
            for row in raw]
    return derive_chain_metadata(rows, spell_id)


def validate_row(spell_id: int, spec: dict, eff: dict) -> dict:
    """Self-checks from the architecture §4 inventory gates."""
    bucket = spec["bucket"]
    kind = spec["expected"]["kind"]
    if bucket not in VALID_BUCKETS:
        raise AssertionError(f"spell {spell_id}: unknown bucket {bucket}")
    if kind not in VALID_EXPECTED:
        raise AssertionError(f"spell {spell_id}: unknown expected {kind}")
    disabled = _query(
        "SELECT COUNT(*) FROM spell_disabled WHERE entry=%d" % spell_id
    )
    if disabled.strip() != "0":
        raise AssertionError(f"spell {spell_id}: listed in spell_disabled")
    row = {
        "class": spec["class"], "spell_id": spell_id,
        "name": eff["name"],
        **chain_metadata(spell_id),
        "bucket": bucket,
        "effect_ids": [eff["effect1"], eff["effect2"], eff["effect3"]],
        "aura_ids": [eff["aura1"], eff["aura2"], eff["aura3"]],
        "target_a": [eff["targ_a1"], eff["targ_a2"], eff["targ_a3"]],
        "target_b": [eff["targ_b1"], eff["targ_b2"], eff["targ_b3"]],
        "power_type": eff["power_type"], "mana_cost": eff["mana_cost"],
        "range_index": eff["range_index"],
        "cast_time_index": eff["cast_time_index"],
        "duration_index": eff["duration_index"], "speed": eff["speed"],
        "stances": eff["stances"], "stances_not": eff["stances_not"],
        "totem": [eff["totem1"], eff["totem2"]],
        "reagents": [eff[f"reagent{i}"] for i in range(1, 9)],
        "equipped_item_class": eff["equipped_item_class"],
        "passive": bool(eff["attributes"] & SPELL_ATTR_PASSIVE),
        "script_name": eff["script_name"],
        "build": eff["build"],
        "expected": spec["expected"],
    }
    if "campaign" in spec:
        row["campaign"] = spec["campaign"]
    return row


def resolve_dbc_dir() -> pathlib.Path:
    raw = os.environ.get(
        "MANGOS_DBC_DIR", "~/azeroth-dev-fixtures/datadir/5875/dbc")
    base = pathlib.Path(raw).expanduser()
    candidates = [base, base / "dbc", base / "5875" / "dbc"]
    for candidate in candidates:
        if (candidate / "Talent.dbc").exists():
            return candidate
    return base


def read_dbc_u32_rows(path: pathlib.Path) -> list[tuple[int, ...]]:
    data = path.read_bytes()
    if len(data) < 20 or data[:4] != b"WDBC":
        raise AssertionError(f"{path}: not a WDBC file")
    _magic, count, fields, record_size, _string_size = struct.unpack_from(
        "<4sIIII", data, 0)
    rows = []
    for index in range(count):
        offset = 20 + index * record_size
        rows.append(struct.unpack_from("<" + "I" * fields, data, offset))
    return rows


def load_talent_tables(dbc_path: pathlib.Path | None = None,
                       ) -> tuple[dict[int, dict], dict[int, int]]:
    dbc_path = dbc_path or resolve_dbc_dir()
    talents = {}
    for row in read_dbc_u32_rows(dbc_path / "Talent.dbc"):
        talents[row[0]] = {
            "tab": row[1], "row": row[2], "col": row[3],
            "rank_spells": list(row[4:9]),
            "depends_on": row[13], "depends_on_rank": row[16],
            "depends_on_spell": row[20],
        }
    tabs = {row[0]: row[12]
            for row in read_dbc_u32_rows(dbc_path / "TalentTab.dbc")}
    return talents, tabs


def parse_talent_builds(text: str) -> list[dict]:
    builds = []
    pattern = re.compile(
        r'TalentBuild \{\s+class: Class::(\w+),\s+role: Role::(\w+),'
        r'\s+name: "([^"]+)",\s+points: &\[(.*?)\],\s+\}', re.S)
    for match in pattern.finditer(text):
        points = [(int(talent_id), int(rank)) for talent_id, rank
                  in re.findall(r"\((\d+),\s*(\d+)\)", match.group(4))]
        builds.append({
            "class": match.group(1), "role": match.group(2),
            "name": match.group(3), "points": points,
        })
    return builds


def expand_talent_build(build: dict, talents: dict[int, dict],
                        tabs: dict[int, int],
                        spell_exists: set[int] | None = None) -> dict:
    """Expand one checked-in build and audit fork talent legality.

    Final known spell per talent is its highest allocated RankID
    (fork rank replacement). Dependency closure is reported as
    required allocations/deficits; missing prerequisites are not
    silently added to the build.
    """
    class_name = build["class"]
    class_id = CLASS_IDS.get(class_name)
    raw_points = [(int(talent_id), int(rank))
                  for talent_id, rank in build["points"]]
    total_points = sum(rank for _talent_id, rank in raw_points)
    issues: list[str] = []
    if total_points != 51:
        issues.append(f"total points {total_points} != 51")
    if class_id is None:
        issues.append(f"unknown class {class_name}")

    allocations: dict[int, int] = {}
    allocation_order: list[int] = []
    seen_counts: dict[int, int] = {}
    for talent_id, rank in raw_points:
        seen_counts[talent_id] = seen_counts.get(talent_id, 0) + 1
        if talent_id not in allocations:
            allocation_order.append(talent_id)
            allocations[talent_id] = rank
        else:
            allocations[talent_id] = max(allocations[talent_id], rank)
    for talent_id, count in sorted(seen_counts.items()):
        if count > 1:
            issues.append(f"duplicate talent {talent_id} entries: {count}")

    expanded_points = []
    known_spell_ids: set[int] = set()
    all_rank_spell_ids: set[int] = set()
    for talent_id in allocation_order:
        rank = allocations[talent_id]
        talent = talents.get(talent_id)
        if talent is None:
            issues.append(f"talent {talent_id} missing from Talent.dbc")
            continue
        tab = int(talent["tab"])
        if class_id is not None:
            class_mask = tabs.get(tab)
            if class_mask is None or not class_mask & (1 << (class_id - 1)):
                issues.append(
                    f"talent {talent_id} tab {tab} is not a {class_name} tab")
        rank_spells = [int(spell) for spell in talent["rank_spells"]]
        nonzero_ranks = [spell for spell in rank_spells if spell]
        if rank < 1 or rank > len(nonzero_ranks):
            issues.append(
                f"talent {talent_id} allocated rank {rank} outside "
                f"available ranks {len(nonzero_ranks)}")
            continue
        allocated_spells = rank_spells[:rank]
        if spell_exists is not None:
            for spell_id in allocated_spells:
                if spell_id not in spell_exists:
                    issues.append(
                        f"talent {talent_id} rank spell {spell_id} "
                        "missing from effective spell_template")
        row = int(talent["row"])
        known_spell_id = allocated_spells[-1]
        known_spell_ids.add(known_spell_id)
        all_rank_spell_ids.update(allocated_spells)
        expanded_points.append({
            "talent_id": talent_id, "allocated_rank": rank,
            "tab": tab, "row": row, "col": int(talent["col"]),
            "rank_spell_ids": allocated_spells,
            "known_spell_id": known_spell_id,
        })

    # Final-allocation tier legality: a legal learning order exists
    # iff every allocated row has at least row*5 points allocated
    # in lower rows of the same tab (fork LearnTalent tier check).
    for talent_id in allocation_order:
        talent = talents.get(talent_id)
        if talent is None:
            continue
        row = int(talent["row"])
        if row == 0:
            continue
        tab = int(talent["tab"])
        lower_points = sum(
            allocated for other_id, allocated in allocations.items()
            if other_id in talents
            and int(talents[other_id]["tab"]) == tab
            and int(talents[other_id]["row"]) < row)
        if lower_points < row * 5:
            issues.append(
                f"talent {talent_id} row {row} has only "
                f"{lower_points} lower-row points in tab {tab}")

    required_allocations = dict(allocations)
    queue = list(allocation_order)
    cursor = 0
    while cursor < len(queue):
        talent_id = queue[cursor]
        cursor += 1
        talent = talents.get(talent_id)
        if talent is None:
            continue
        depends_on = int(talent.get("depends_on", 0))
        if not depends_on:
            continue
        required_rank = int(talent.get("depends_on_rank", 0)) + 1
        if required_allocations.get(depends_on, 0) < required_rank:
            required_allocations[depends_on] = required_rank
            queue.append(depends_on)
    dependency_deficits = {
        str(talent_id): required - allocations.get(talent_id, 0)
        for talent_id, required in sorted(required_allocations.items())
        if required > allocations.get(talent_id, 0)
    }
    if dependency_deficits:
        issues.append(
            "dependency deficits: "
            + ", ".join(f"{tid} needs +{deficit}"
                        for tid, deficit in dependency_deficits.items()))

    required_spell_ids = sorted({
        int(talents[talent_id]["depends_on_spell"])
        for talent_id in required_allocations
        if talent_id in talents and talents[talent_id].get("depends_on_spell")
    })
    closure_known_spell_ids: set[int] = set()
    for talent_id, rank in required_allocations.items():
        talent = talents.get(talent_id)
        if talent is None:
            issues.append(f"prerequisite talent {talent_id} missing")
            continue
        rank_spells = [int(spell) for spell in talent["rank_spells"] if spell]
        if 1 <= rank <= len(rank_spells):
            closure_known_spell_ids.add(rank_spells[rank - 1])
        else:
            issues.append(
                f"prerequisite talent {talent_id} required rank {rank} "
                "outside available ranks")

    return {
        "class": class_name, "role": build["role"], "name": build["name"],
        "total_points": total_points,
        "closure_total_points": sum(required_allocations.values()),
        "legal_51": not issues and not dependency_deficits,
        "issues": issues,
        "dependency_deficits": dependency_deficits,
        "required_spell_ids": required_spell_ids,
        "points": expanded_points,
        "known_spell_ids": sorted(known_spell_ids),
        "all_rank_spell_ids": sorted(all_rank_spell_ids),
        "closure_known_spell_ids": sorted(closure_known_spell_ids),
    }


def select_talent_builds(expanded: list[dict]) -> dict[str, dict | None]:
    selected: dict[str, dict | None] = {}
    for class_name in CLASS_IDS:
        chosen = None
        for role in PREFERRED_TALENT_ROLES:
            for build in expanded:
                if (build["class"] == class_name and build["role"] == role
                        and build["legal_51"]):
                    chosen = build
                    break
            if chosen is not None:
                break
        selected[class_name] = None if chosen is None else {
            "name": chosen["name"], "role": chosen["role"],
            "total_points": chosen["total_points"],
            "known_spell_ids": chosen["known_spell_ids"],
            "required_spell_ids": chosen["required_spell_ids"],
        }
    return selected


def effective_spell_ids() -> set[int]:
    rows = _query_rows(
        "SELECT entry FROM spell_template t1 WHERE build="
        "(SELECT max(build) FROM spell_template t2 "
        f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})")
    return {int(row[0]) for row in rows if row}


def generate_talent_builds() -> dict:
    builds = parse_talent_builds(TALENT_BUILDS_RS.read_text())
    if len(builds) != 26:
        raise AssertionError(f"expected 26 talent builds, parsed {len(builds)}")
    talents, tabs = load_talent_tables()
    spell_ids = effective_spell_ids()
    expanded = [expand_talent_build(build, talents, tabs, spell_ids)
                for build in builds]
    selected = select_talent_builds(expanded)
    unresolved = [name for name, build in selected.items() if build is None]
    return {
        "source": str(TALENT_BUILDS_RS),
        "dbc_dir": str(resolve_dbc_dir()),
        "build_count": len(expanded),
        "selected": selected,
        "legal_classes": [name for name, build in selected.items()
                          if build is not None],
        "unresolved_classes": unresolved,
        "builds": expanded,
    }


def talent_build_summary(doc: dict) -> str:
    unresolved = doc.get("unresolved_classes", [])
    if unresolved:
        return ("enumerated 26 checked-in builds; legal selected builds "
                f"for {len(doc.get('legal_classes', []))}/9 classes; "
                f"unresolved classes: {', '.join(unresolved)} "
                "(checked-in builds fail fork DependsOn/tier legality); "
                "fan-out blocked")
    return "resolved: legal selected 51-point build for every class"


def load_skill_race_class_info(dbc_path: pathlib.Path | None = None,
                               ) -> tuple[dict[int, list[dict]], dict]:
    dbc_path = dbc_path or resolve_dbc_dir()
    skill_line_ids = {row[0] for row in read_dbc_u32_rows(
        dbc_path / "SkillLine.dbc")}
    raw_rows = read_dbc_u32_rows(dbc_path / "SkillRaceClassInfo.dbc")
    by_skill: dict[int, list[dict]] = {}
    for row in raw_rows:
        skill_id = row[1]
        if skill_id not in skill_line_ids:
            continue
        by_skill.setdefault(skill_id, []).append({
            "skill_id": skill_id, "race_mask": row[2],
            "class_mask": row[3], "flags": row[4],
            "req_level": row[5], "skill_tier_id": row[6],
        })
    return by_skill, {
        "raw_rows": len(raw_rows),
        "filtered_rows": sum(len(rows) for rows in by_skill.values()),
        "skill_line_rows": len(skill_line_ids),
    }


def spell_fit_by_class_and_race(spell_id: int, race: int, class_id: int,
                                level: int,
                                sla_by_spell: dict[int, list[dict]],
                                rc_by_skill: dict[int, list[dict]]) -> bool:
    """Fork `Player::IsSpellFitByClassAndRace` train mode."""
    race_mask = 1 << (race - 1)
    class_mask = 1 << (class_id - 1)
    abilities = sla_by_spell.get(spell_id, [])
    if not abilities:
        return True
    for ability in abilities:
        if ability["race_mask"] and not ability["race_mask"] & race_mask:
            continue
        if ability["class_mask"] and not ability["class_mask"] & class_mask:
            continue
        for entry in rc_by_skill.get(int(ability["skill_id"]), []):
            if entry["race_mask"] & race_mask and entry["class_mask"] & class_mask:
                if entry["flags"] & ABILITY_SKILL_NONTRAINABLE:
                    return False
                if entry["req_level"] and level < entry["req_level"]:
                    return False
        return True
    return False


def generate_g1_audit() -> dict:
    effective = {
        int(row[0]): {"spell_level": int(row[1]), "family": int(row[2])}
        for row in _query_rows(
            "SELECT entry,spellLevel,spellFamilyName "
            "FROM spell_template t1 WHERE build="
            "(SELECT max(build) FROM spell_template t2 "
            f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})")
        if row
    }
    sla_rows = []
    sla_by_spell: dict[int, list[dict]] = {}
    for row in _query_rows(
            "SELECT skill_id,spell_id,race_mask,class_mask "
            f"FROM skill_line_ability WHERE build={SUPPORTED_BUILD} "
            "ORDER BY id"):
        ability = {
            "skill_id": int(row[0]), "spell_id": int(row[1]),
            "race_mask": int(row[2]), "class_mask": int(row[3]),
        }
        sla_rows.append(ability)
        sla_by_spell.setdefault(ability["spell_id"], []).append(ability)

    talents, _tabs = load_talent_tables()
    talent_spell_ids = {
        int(spell) for talent in talents.values()
        for spell in talent["rank_spells"] if spell
    }
    rc_by_skill, rc_counts = load_skill_race_class_info()
    races_by_class: dict[int, list[int]] = {}
    for row in _query_rows(
            "SELECT race,class FROM playercreateinfo ORDER BY race,class"):
        races_by_class.setdefault(int(row[1]), []).append(int(row[0]))

    classes = {}
    for class_name, class_id in CLASS_IDS.items():
        class_bit = 1 << (class_id - 1)
        pre_g1: set[int] = set()
        for ability in sla_rows:
            if ability["class_mask"] and not ability["class_mask"] & class_bit:
                continue
            spell_id = ability["spell_id"]
            spell = effective.get(spell_id)
            if spell is None or spell_id in talent_spell_ids:
                continue
            if not 1 <= spell["spell_level"] <= 60:
                continue
            if spell["family"] != CLASS_FAMILIES[class_name]:
                continue
            pre_g1.add(spell_id)
        races = races_by_class.get(class_id, [])
        per_race = {}
        post_g1: set[int] = set()
        for race in races:
            fit = {
                spell_id for spell_id in pre_g1
                if spell_fit_by_class_and_race(
                    spell_id, race, class_id, 60,
                    sla_by_spell, rc_by_skill)
            }
            per_race[str(race)] = len(fit)
            post_g1.update(fit)
        classes[class_name] = {
            "pre_g1_count": len(pre_g1),
            "post_g1_count": len(post_g1),
            "excluded_spell_ids": sorted(pre_g1 - post_g1),
            "per_race_counts": per_race,
        }
    unresolved = [name for name, data in classes.items()
                  if data["excluded_spell_ids"]]
    return {
        "level": 60,
        "skill_race_class_info": rc_counts,
        "classes": classes,
        "unresolved_classes": unresolved,
        "resolved": not unresolved,
    }


def g1_summary(doc: dict) -> str:
    counts = ", ".join(
        f"{name} {data['post_g1_count']}"
        for name, data in doc.get("classes", {}).items())
    if doc.get("resolved"):
        return f"resolved: generator replay 0 exclusions ({counts})"
    return (f"unresolved: generator replay exclusions for "
            f"{', '.join(doc.get('unresolved_classes', []))} ({counts})")


def generate_pilot() -> dict:
    rows = []
    for spell_id, spec in sorted(PILOT_ROWS.items()):
        eff = effective_row(spell_id)
        rows.append(validate_row(spell_id, spec, eff))
    rows.sort(key=lambda r: (r["class"], r["spell_id"]))
    by_class: dict[str, int] = {}
    for row in rows:
        by_class[row["class"]] = by_class.get(row["class"], 0) + 1
    try:
        g1_doc = generate_g1_audit()
        g1_text = g1_summary(g1_doc)
        g1_builds = {
            "resolved": g1_doc["resolved"],
            "classes": g1_doc["classes"],
            "skill_race_class_info": g1_doc["skill_race_class_info"],
            "artifact": G1_AUDIT_PATH.name,
        }
    except Exception as exc:  # pilot rows themselves need no G1 set
        g1_text = f"unavailable: {type(exc).__name__}: {exc}"
        g1_builds = {"resolved": False, "classes": {},
                     "artifact": G1_AUDIT_PATH.name}
    try:
        talent_doc = generate_talent_builds()
        talent_summary = talent_build_summary(talent_doc)
        talent_builds = {
            "selected": talent_doc["selected"],
            "legal_classes": talent_doc["legal_classes"],
            "unresolved_classes": talent_doc["unresolved_classes"],
            "build_count": talent_doc["build_count"],
            "artifact": TALENT_BUILDS_PATH.name,
        }
    except Exception as exc:  # pilot rows themselves need no talents
        talent_summary = f"unavailable: {type(exc).__name__}: {exc}"
        talent_builds = {
            "selected": {}, "legal_classes": [],
            "unresolved_classes": list(CLASS_IDS), "build_count": 0,
            "artifact": TALENT_BUILDS_PATH.name,
        }
    return {
        "mode": "pilot",
        "supported_build": SUPPORTED_BUILD,
        "per_class_counts": by_class,
        "g1_skillraceclassinfo": g1_text,
        "g1_audit": g1_builds,
        "g2_talent_builds": talent_summary,
        "talent_builds": talent_builds,
        "g3_isspellvalid": "caveat: full IsSpellValid replay not rerun",
        "g4_script_evidence": "caveat: SCRIPT_EFFECT rows ride as "
        "BLOCKED script-evidence-unknown",
        "g5_trainer_equivalence": "caveat: bench-vs-production trainer "
        "equivalence unverified",
        "rows": rows,
    }


def run_self_test() -> int:
    # Bucket/expected enums are closed; every pilot row uses one each.
    for spell_id, spec in PILOT_ROWS.items():
        assert spec["bucket"] in VALID_BUCKETS, spell_id
        assert spec["expected"]["kind"] in VALID_EXPECTED, spell_id
    # Rank metadata is derived from spell_chain, never trusted from the
    # hand-pinned pilot spec. A truncated chain is the regression this
    # guards (the shipped generator discarded its own chain query).
    synthetic = [
        {"spell_id": 145, "prev_spell": 143, "first_spell": 133,
         "rank": 3, "req_spell": 0},
        {"spell_id": 133, "prev_spell": 0, "first_spell": 133,
         "rank": 1, "req_spell": 0},
        {"spell_id": 143, "prev_spell": 133, "first_spell": 133,
         "rank": 2, "req_spell": 0},
    ]
    meta = derive_chain_metadata(synthetic, 143)
    assert meta == {
        "chain_first": 133, "rank": 2,
        "all_rank_ids": [133, 143, 145], "last_rank_id": 145,
        "rank_count": 3, "is_last_rank": False,
    }, meta
    assert derive_chain_metadata([], 6795) == {
        "chain_first": 6795, "rank": 1, "all_rank_ids": [6795],
        "last_rank_id": 6795, "rank_count": 1, "is_last_rank": True,
    }
    try:
        derive_chain_metadata([
            {"spell_id": 133, "prev_spell": 0, "first_spell": 133,
             "rank": 1, "req_spell": 0},
            {"spell_id": 143, "prev_spell": 999, "first_spell": 133,
             "rank": 2, "req_spell": 0},
        ], 143)
    except AssertionError:
        pass
    else:
        raise AssertionError("broken prev_spell chain accepted")
    # Matrix artifact, if present, must obey the architecture §4 schema.
    if MATRIX_PATH.exists():
        doc = json.loads(MATRIX_PATH.read_text())
        assert doc["supported_build"] == SUPPORTED_BUILD
        seen = set()
        for row in doc["rows"]:
            for key in ("class", "spell_id", "name", "bucket", "expected",
                        "chain_first", "rank", "all_rank_ids",
                        "last_rank_id", "rank_count", "is_last_rank",
                        "effect_ids", "aura_ids", "target_a", "target_b",
                        "power_type", "mana_cost", "range_index",
                        "cast_time_index", "duration_index", "speed",
                        "stances", "stances_not", "totem", "reagents",
                        "equipped_item_class", "passive", "script_name",
                        "build"):
                assert key in row, (row.get("spell_id"), key)
            assert row["name"], row
            assert row["bucket"] in VALID_BUCKETS, row
            assert row["expected"]["kind"] in VALID_EXPECTED, row
            assert row["all_rank_ids"][row["rank"] - 1] == row["spell_id"], row
            assert row["chain_first"] == row["all_rank_ids"][0], row
            assert row["last_rank_id"] == row["all_rank_ids"][-1], row
            assert row["rank_count"] == len(row["all_rank_ids"]), row
            assert row["is_last_rank"] is (
                row["spell_id"] == row["last_rank_id"]), row
            assert (row["class"], row["spell_id"]) not in seen, row
            seen.add((row["class"], row["spell_id"]))
    # G1 artifact, if generated, must reproduce the derived
    # matrix's exact level-60 union counts with no exclusions.
    if G1_AUDIT_PATH.exists():
        g1_doc = json.loads(G1_AUDIT_PATH.read_text())
        expected_g1 = {
            "Warrior": 121, "Paladin": 160, "Hunter": 123,
            "Rogue": 89, "Priest": 214, "Shaman": 159,
            "Mage": 176, "Warlock": 202, "Druid": 187,
        }
        assert g1_doc["resolved"] is True
        for class_name, expected in expected_g1.items():
            data = g1_doc["classes"][class_name]
            assert data["pre_g1_count"] == expected, (class_name, data)
            assert data["post_g1_count"] == expected, (class_name, data)
            assert data["excluded_spell_ids"] == [], (class_name, data)
    # Talent artifact, if generated, must keep the audited shape.
    if TALENT_BUILDS_PATH.exists():
        talent_doc = json.loads(TALENT_BUILDS_PATH.read_text())
        assert talent_doc["build_count"] == 26, talent_doc["build_count"]
        assert len(talent_doc["builds"]) == 26
        assert set(talent_doc["selected"]) == set(CLASS_IDS)
        for build in talent_doc["builds"]:
            assert build["total_points"] == 51, build["name"]
            if build["legal_51"]:
                assert not build["issues"], build["name"]
                assert build["dependency_deficits"] == {}, build["name"]
    # G1 fit replay mirrors fork train mode exactly: no SLA row
    # fits by default; a matching SkillRaceClassInfo row can
    # reject for NONTRAINABLE or reqLevel, but a non-matching RC
    # row alone does not reject the first matching SLA row.
    synthetic_sla = {
        100: [{"skill_id": 10, "spell_id": 100, "race_mask": 0,
               "class_mask": 1}],
        101: [{"skill_id": 10, "spell_id": 101, "race_mask": 0,
               "class_mask": 1}],
    }
    synthetic_rc = {10: [{"race_mask": 1, "class_mask": 1,
                          "flags": 0, "req_level": 40}]}
    assert spell_fit_by_class_and_race(
        999, 1, 1, 60, synthetic_sla, synthetic_rc) is True
    assert spell_fit_by_class_and_race(
        100, 1, 1, 60, synthetic_sla, synthetic_rc) is True
    assert spell_fit_by_class_and_race(
        100, 1, 1, 39, synthetic_sla, synthetic_rc) is False
    assert spell_fit_by_class_and_race(
        100, 2, 1, 60, synthetic_sla, synthetic_rc) is True
    synthetic_rc[10][0]["flags"] = 0x100
    assert spell_fit_by_class_and_race(
        100, 1, 1, 60, synthetic_sla, synthetic_rc) is False
    # Talent-build expansion: checked-in points are (talent id,
    # ranks); final known spell is the highest allocated rank, and
    # DependsOn closure deficits are explicit, never silently filled.
    parsed = parse_talent_builds("""
    TalentBuild {
        class: Class::Warrior,
        role: Role::MeleeDps,
        name: "synthetic",
        points: &[(1, 2), (2, 1)],
    },
    """)
    assert parsed == [{
        "class": "Warrior", "role": "MeleeDps", "name": "synthetic",
        "points": [(1, 2), (2, 1)],
    }]
    synthetic_talents = {
        1: {"tab": 10, "row": 0, "col": 0,
            "rank_spells": [101, 102, 0, 0, 0],
            "depends_on": 0, "depends_on_rank": 0,
            "depends_on_spell": 0},
        2: {"tab": 10, "row": 1, "col": 0,
            "rank_spells": [201, 0, 0, 0, 0],
            "depends_on": 1, "depends_on_rank": 1,
            "depends_on_spell": 339},
    }
    synthetic_tabs = {10: 1}
    expanded = expand_talent_build(
        parsed[0], synthetic_talents, synthetic_tabs,
        spell_exists={101, 102, 201})
    assert expanded["total_points"] == 3, expanded
    assert expanded["known_spell_ids"] == [102, 201], expanded
    assert expanded["all_rank_spell_ids"] == [101, 102, 201], expanded
    assert expanded["dependency_deficits"] == {}, expanded
    assert expanded["required_spell_ids"] == [339], expanded
    assert expanded["legal_51"] is False, expanded
    missing = expand_talent_build(
        {"class": "Warrior", "role": "MeleeDps", "name": "missing",
         "points": [(2, 1)]}, synthetic_talents, synthetic_tabs,
        spell_exists={101, 102, 201})
    assert missing["dependency_deficits"] == {"1": 2}, missing
    assert missing["legal_51"] is False and missing["issues"], missing
    duplicate = expand_talent_build(
        {"class": "Warrior", "role": "MeleeDps", "name": "dup",
         "points": [(1, 1), (1, 1)]}, synthetic_talents, synthetic_tabs,
        spell_exists={101, 102, 201})
    assert any("duplicate" in issue for issue in duplicate["issues"])
    # Wire-shape constants the harness depends on.
    assert (0xF130 << 48) | (705 << 24) | 344 == 0xF1300002C1000158
    print("SELFTEST matrix enums + schema + guid math ok")
    print("SPELL-MATRIX-GEN-SELFTEST-PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pilot", action="store_true",
                        help="emit the pinned pilot matrix")
    parser.add_argument("--full", action="store_true",
                        help="whole-matrix fan-out (gated on G1/G2)")
    parser.add_argument("--g1", action="store_true",
                        help="replay the SkillRaceClassInfo train-mode gate")
    parser.add_argument("--talent-builds", action="store_true",
                        help="enumerate/audit checked-in 51-point builds")
    parser.add_argument("--self-test", action="store_true",
                        help="offline invariant checks, no DB needed")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    if args.g1:
        doc = generate_g1_audit()
        G1_AUDIT_PATH.write_text(json.dumps(doc, indent=1) + "\n")
        print(f"G1 {g1_summary(doc)} -> {G1_AUDIT_PATH}")
        return 0 if doc["resolved"] else 2
    if args.talent_builds:
        doc = generate_talent_builds()
        TALENT_BUILDS_PATH.write_text(json.dumps(doc, indent=1) + "\n")
        print(f"TALENT-BUILDS {talent_build_summary(doc)} "
              f"-> {TALENT_BUILDS_PATH}")
        for class_name, selected in doc["selected"].items():
            print(f"SELECTED {class_name}: "
                  f"{selected['name'] if selected else 'NONE'}")
        return 2 if doc["unresolved_classes"] else 0
    if args.full and not args.pilot:
        print("spell_matrix_gen: --full blocked: G1/G2 pre-fan-out gates "
              "not yet closed in this generator (see architecture §4)",
              file=sys.stderr)
        return 2
    doc = generate_pilot()
    MATRIX_PATH.write_text(json.dumps(doc, indent=1) + "\n")
    print(f"MATRIX-GEN-OK mode=pilot rows={len(doc['rows'])} "
          f"classes={sorted(doc['per_class_counts'])} -> {MATRIX_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
