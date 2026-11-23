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

`--full --class <Class>` (per-class fan-out over the pinned classes
Paladin and Druid): emit `spell_campaign_matrix-<Class>.json` with
one cast row per merged S7 component over the G1 union plus the
selected 51-point talent build's known spells, plus the all-rank
seed list. Full mode never writes MATRIX_PATH, so `--pilot` output
stays byte-stable.
Whole-matrix (9-class) fan-out stays closed until G2 has a legal
selected build for every class.
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


def class_matrix_path(class_name: str) -> pathlib.Path:
    """Per-class fan-out artifact; full mode never touches MATRIX_PATH."""
    return SCRIPT_DIR / f"spell_campaign_matrix-{class_name}.json"


TALENT_BUILDS_PATH = SCRIPT_DIR / "spell_campaign_talent_builds.json"
G1_AUDIT_PATH = SCRIPT_DIR / "spell_campaign_g1.json"
TALENT_BUILDS_RS = SCRIPT_DIR.parent / "crates" / "mangos-bots" / "src" / "talent_builds.rs"
CLASS_IDS = {
    "Warrior": 1,
    "Paladin": 2,
    "Hunter": 3,
    "Rogue": 4,
    "Priest": 5,
    "Shaman": 7,
    "Mage": 8,
    "Warlock": 9,
    "Druid": 11,
}
PREFERRED_TALENT_ROLES = ("MeleeDps", "RangeDps", "Tank", "Healer")
CLASS_FAMILIES = {
    "Warrior": 4,
    "Paladin": 10,
    "Hunter": 9,
    "Rogue": 8,
    "Priest": 6,
    "Shaman": 11,
    "Mage": 3,
    "Warlock": 5,
    "Druid": 7,
}
ABILITY_SKILL_NONTRAINABLE = 0x100

# --- Full-fan-out derivation (S2/S7 + SPELL_MATRIX.md §3) -------------------
# Bucket key = first match in this order (each spell gets exactly one
# primary bucket). Target vocabulary from SpellDefines.h (SpellTarget),
# effect vocabulary from SpellEffects, aura vocabulary from
# SpellAuraDefines.h.

# T25 Holy Shock ranks share TARGET_UNIT(25) and need both a friendly
# heal leg and a hostile damage leg; they get their own bucket until
# both legs pass.
PALADIN_DUAL_IDS = {20929, 20930}

SELF_TARGETS = {1, 3, 4, 27}
FRIENDLY_TARGETS = {21, 45, 29, 30, 31, 59}
HOSTILE_TARGETS = {6, 2, 15, 16, 24, 28, 36, 54}
PET_TARGETS = {5}
PARTY_TARGETS = {35, 37, 57, 58, 61}
LOCATION_TARGETS = {9, 10, 17, 18, 22, 39, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 53, 55, 63}
AOE_TARGETS = {7, 8, 15, 16, 24, 28, 29, 30, 31, 33, 34, 36, 51, 52, 56, 59, 60}

RESURRECT_EFFECTS = {18, 113}
SUMMON_OBJECT_EFFECTS = {76, 86, 104, 105, 106, 107}
GROUND_EFFECTS = {27, 32} | SUMMON_OBJECT_EFFECTS
PET_SUMMON_EFFECTS = {28, 56, 112}
LEARN_PET_EFFECTS = {57}
DUEL_EFFECTS = {83}
HEAL_EFFECTS = {10, 62, 67}
SCHOOL_DAMAGE_EFFECTS = {2}
APPLY_AURA_EFFECTS = {6, 35}
DISPEL_EFFECTS = {38}
THREAT_EFFECTS = {63}
SCRIPT_EFFECTS = {77}
#: SPELL_EFFECT_TELEPORT_UNITS (fork `SpellEffects.cpp:1536`): a
#: teleport-only row has no object/damage/heal observable on the
#: bench map; the cross-map leg verifies the transfer instead.
TELEPORT_EFFECTS = {5}
MOD_SHAPESHIFT_AURA = 36
#: `SPELL_ATTR_EX_FINISHING_MOVE_DAMAGE|DURATION` (`SpellDefines.h`):
#: the fork's `NeedsComboPoints` predicate.
FINISHING_MOVE_EX = 0x00100000 | 0x00400000
#: `SPELL_EFFECT_ADD_COMBO_POINTS`.
ADD_COMBO_POINTS_EFFECT = 80
USES_RANGED_SLOT_ATTR = 0x2

# Expected Paladin full-160 bucket distribution (prior audit over the
# exact G1 union; item = 9 reagent + 2 equip). derive_full_rows asserts
# this so derivation drift fails loudly instead of silently.
PALADIN_FULL_BUCKET_COUNTS = {
    "self": 50,
    "friendly": 48,
    "hostile_dummy": 39,
    "item_reagent_ammo": 11,
    "party_duel": 6,
    "ground_area": 4,
    "dual": 2,
    "corpse": 0,
    "pet": 0,
    "form_totem": 0,
    "passive": 0,
}

#: Fork stance gate (`SpellEntry::GetErrorAtShapeshiftedCast`,
#: `SpellEntry.cpp`): at form 0 (the harness's rest state) stanceMask
#: is 0, so StancesNot never blocks and only Stances != 0 blocks (as
#: ONLY_SHAPESHIFT), unless SPELL_ATTR_EX2_ALLOW_WHILE_NOT_SHAPESHIFTED
#: is set. Druid balance/resto rows carry Stances=<Moonkin bit, 1<<30>
#: *with* that flag, so they cast from caster form; Bear/Cat-only rows
#: without it do not. A Stances bit k means form k+1 (`SharedDefines.h`
#: ShapeshiftForm: Cat 1, Bear 5, DireBear 8, Moonkin 0x1F).
ALLOW_WHILE_NOT_SHAPESHIFTED_EX2 = 0x00080000

#: Per-class fan-out pins (G1 union size, selected-build known count,
#: G1/talent overlap, merged-extra seed exclusions). derive_full_rows
#: asserts these for pinned classes so derivation drift fails loudly;
#: unpinned classes run without the asserts. Paladin is the
#: reproduction case: its entries are the exact pre-generalization
#: literals.
CLASS_FANOUT_EXPECTATIONS = {
    "Paladin": {
        "union": 160,
        "talent_known": 14,
        "overlap": [],
        "excluded": [20473, 20911, 20925, 26573],
    },
    "Druid": {
        # Excluded are shipped-chain lower ranks outside both audited
        # sets that carry a talent cost (Insect Swarm 5570 and Feral
        # Faerie Fire 16857, rank 1 each): the Paladin 20473-shaped
        # merged extra, keeping the seed at the 51 cap.
        "union": 187,
        "talent_known": 15,
        "overlap": [],
        "excluded": [5570, 16857],
    },
}

#: Per-class G1-union bucket distributions under derive_bucket's
#: first-match order. Zero-filled so a new bucket appearing for a
#: pinned class fails loudly.
CLASS_FULL_BUCKET_COUNTS = {
    "Paladin": PALADIN_FULL_BUCKET_COUNTS,
    "Druid": {
        "self": 11,
        "friendly": 48,
        "hostile_dummy": 45,
        "item_reagent_ammo": 2,
        "party_duel": 0,
        "ground_area": 4,
        "dual": 0,
        "corpse": 5,
        "pet": 0,
        "form_totem": 64,
        "passive": 3,
        "form": 5,
    },
}

# Buckets and expected outcomes below were derived from
# SPELL_MATRIX.md S1-S12 and verified against the oracle bodies named
# there. Rank metadata is deliberately absent here: the generator
# derives it from spell_chain and asserts each row still resolves in
# the bench DB before emitting it.
PILOT_ROWS: dict[int, dict] = {
    5242: {
        "class": "Warrior",
        "bucket": "self",
        "expected": {"kind": "aura_on_caster", "aura": 5242},
    },
    6546: {
        "class": "Warrior",
        "bucket": "hostile_dummy",
        "expected": {"kind": "health_delta_target", "aura": 6546},
    },
    168: {
        "class": "Mage",
        "bucket": "self",
        "expected": {"kind": "aura_on_caster", "aura": 168},
    },
    133: {
        "class": "Mage",
        "bucket": "hostile_dummy",
        "expected": {"kind": "damage_log", "opcode": 592},
    },
    5187: {
        # Rank 3, not rank 1: a rank-1 heal (oracle band 40-54) sits
        # inside the bench regen-tick noise (+34/1.5s, measured), so a
        # single window cannot separate it from a phase-aligned tick.
        # Rank 3 separates completely. This is a pinned pilot probe,
        # not the chain's last rank; generated metadata says so.
        "class": "Druid",
        "bucket": "friendly",
        "expected": {"kind": "heal_delta_target", "heal_band": [204, 254], "observed_jump_band": [170, 288]},
    },
    5487: {
        "class": "Druid",
        "bucket": "form",
        "expected": {"kind": "form_change", "aura": 5487},
    },
    768: {
        "class": "Druid",
        "bucket": "form",
        "expected": {"kind": "form_change", "aura": 768},
    },
    6795: {
        "class": "Druid",
        "bucket": "form",
        "expected": {"kind": "expect_cast_failure", "note": "Growl in caster form; stance gate or failure"},
        "campaign": "classify_only_wrong_form",
    },
    20484: {
        "class": "Druid",
        "bucket": "corpse",
        "expected": {"kind": "resurrect_request"},
        "campaign": "classify_only_corpse",
    },
    8921: {
        "class": "Druid",
        "bucket": "hostile_dummy",
        "expected": {"kind": "health_delta_target", "aura": 8921},
    },
}

VALID_BUCKETS = {
    "self",
    "friendly",
    "hostile_dummy",
    "pet",
    "corpse",
    "ground_area",
    "item_reagent_ammo",
    "party_duel",
    "form_totem",
    "form",
    "passive",
    "dual",
}
VALID_EXPECTED = {
    "aura_on_caster",
    "aura_on_target",
    "health_delta_target",
    "heal_delta_target",
    "damage_log",
    "object_created",
    "pet_created",
    "form_change",
    "item_consumed",
    "learned_spell",
    "resurrect_request",
    "teleport",
    "none_marker",
    "expect_cast_failure",
    "dual_outcome",
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
        ["mysql", "-h", "127.0.0.1", "-u", "mangos", f"-p{_bench_password()}", "mangos", "-N", "-e", stmt],
        capture_output=True,
        text=True,
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
        ("entry", int),
        ("build", int),
        ("name", str),
        ("attributes", int),
        ("attributes_ex", int),
        ("attributes_ex2", int),
        ("power_type", int),
        ("mana_cost", int),
        ("range_index", int),
        ("cast_time_index", int),
        ("duration_index", int),
        ("speed", float),
        ("stances", int),
        ("stances_not", int),
        ("totem1", int),
        ("totem2", int),
        ("reagent1", int),
        ("reagent2", int),
        ("reagent3", int),
        ("reagent4", int),
        ("reagent5", int),
        ("reagent6", int),
        ("reagent7", int),
        ("reagent8", int),
        ("equipped_item_class", int),
        ("effect1", int),
        ("effect2", int),
        ("effect3", int),
        ("aura1", int),
        ("aura2", int),
        ("aura3", int),
        ("targ_a1", int),
        ("targ_a2", int),
        ("targ_a3", int),
        ("targ_b1", int),
        ("targ_b2", int),
        ("targ_b3", int),
        ("recovery_time", int),
        ("effect_base_points1", int),
        ("effect_base_points2", int),
        ("effect_base_points3", int),
        ("effect_die_sides1", int),
        ("effect_die_sides2", int),
        ("effect_die_sides3", int),
        ("effect_base_dice1", int),
        ("effect_base_dice2", int),
        ("effect_base_dice3", int),
        ("script_name", str),
    ]
    out = _query(
        "SELECT entry,build,name,attributes,attributesEx,attributesEx2,"
        "powerType,manaCost,"
        "rangeIndex,castingTimeIndex,durationIndex,speed,stances,"
        "stancesNot,totem1,totem2,reagent1,reagent2,reagent3,"
        "reagent4,reagent5,reagent6,reagent7,reagent8,"
        "equippedItemClass,effect1,effect2,effect3,"
        "effectApplyAuraName1,effectApplyAuraName2,"
        "effectApplyAuraName3,effectImplicitTargetA1,"
        "effectImplicitTargetA2,effectImplicitTargetA3,"
        "effectImplicitTargetB1,effectImplicitTargetB2,"
        "effectImplicitTargetB3,recoveryTime,"
        "effectBasePoints1,effectBasePoints2,effectBasePoints3,"
        "effectDieSides1,effectDieSides2,effectDieSides3,"
        "effectBaseDice1,effectBaseDice2,effectBaseDice3,script_name "
        f"FROM spell_template t1 WHERE entry={spell_id} AND "
        f"build=(SELECT max(build) FROM spell_template t2 "
        f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})"
    )
    if not out:
        raise AssertionError(f"spell {spell_id}: no effective row at build <={SUPPORTED_BUILD}")
    cols = out.split("\t")
    if len(cols) == len(fields) - 1:
        # _query strips the trailing tab of an empty script_name.
        cols.append("")
    if len(cols) != len(fields):
        raise AssertionError(f"spell {spell_id}: effective row has {len(cols)} fields, want {len(fields)}")
    row = {key: conv(value) for (key, conv), value in zip(fields, cols)}
    # Keep the old short keys for any local probe importing this helper.
    row.update(
        {
            "e1": row["effect1"],
            "e2": row["effect2"],
            "e3": row["effect3"],
            "targA1": row["targ_a1"],
            "targB1": row["targ_b1"],
            "durationIndex": row["duration_index"],
            "rangeIndex": row["range_index"],
        }
    )
    return row


def derive_chain_metadata(chain_rows: list[dict], spell_id: int) -> dict:
    """Resolve and validate one spell_chain in S7 rank order."""
    if not chain_rows:
        return {
            "chain_first": spell_id,
            "rank": 1,
            "all_rank_ids": [spell_id],
            "last_rank_id": spell_id,
            "rank_count": 1,
            "is_last_rank": True,
        }
    rows = sorted(chain_rows, key=lambda r: int(r["rank"]))
    first = int(rows[0]["first_spell"])
    all_rank_ids = [int(r["spell_id"]) for r in rows]
    if all_rank_ids.count(spell_id) != 1:
        raise AssertionError(f"spell {spell_id}: chain {first} does not contain it once")
    for index, chain_row in enumerate(rows, start=1):
        if int(chain_row["first_spell"]) != first:
            raise AssertionError(f"spell chain {first}: mixed first_spell")
        if int(chain_row["rank"]) != index:
            raise AssertionError(f"spell chain {first}: rank gap at {index}")
        expected_prev = 0 if index == 1 else all_rank_ids[index - 2]
        if int(chain_row["prev_spell"]) != expected_prev:
            raise AssertionError(
                f"spell chain {first}: prev_spell at rank {index} is {chain_row['prev_spell']}, want {expected_prev}"
            )
    rank = all_rank_ids.index(spell_id) + 1
    return {
        "chain_first": first,
        "rank": rank,
        "all_rank_ids": all_rank_ids,
        "last_rank_id": all_rank_ids[-1],
        "rank_count": len(all_rank_ids),
        "is_last_rank": spell_id == all_rank_ids[-1],
    }


def chain_metadata(spell_id: int) -> dict:
    selected = _query_rows(
        "SELECT spell_id,prev_spell,first_spell,`rank`,req_spell "
        f"FROM spell_chain WHERE spell_id={spell_id} "
        f"AND build_min <= {SUPPORTED_BUILD} "
        f"AND build_max >= {SUPPORTED_BUILD}"
    )
    if not selected:
        return derive_chain_metadata([], spell_id)
    if len(selected) != 1:
        raise AssertionError(f"spell {spell_id}: {len(selected)} active spell_chain rows")
    first = int(selected[0][2])
    raw = _query_rows(
        "SELECT spell_id,prev_spell,first_spell,`rank`,req_spell "
        f"FROM spell_chain WHERE first_spell={first} "
        f"AND build_min <= {SUPPORTED_BUILD} "
        f"AND build_max >= {SUPPORTED_BUILD}"
    )
    keys = ("spell_id", "prev_spell", "first_spell", "rank", "req_spell")
    rows = [{key: int(value) for key, value in zip(keys, row)} for row in raw]
    return derive_chain_metadata(rows, spell_id)


def validate_row(spell_id: int, spec: dict, eff: dict) -> dict:
    """Self-checks from the architecture §4 inventory gates."""
    bucket = spec["bucket"]
    kind = spec["expected"]["kind"]
    if bucket not in VALID_BUCKETS:
        raise AssertionError(f"spell {spell_id}: unknown bucket {bucket}")
    if kind not in VALID_EXPECTED:
        raise AssertionError(f"spell {spell_id}: unknown expected {kind}")
    disabled = _query("SELECT COUNT(*) FROM spell_disabled WHERE entry=%d" % spell_id)
    if disabled.strip() != "0":
        raise AssertionError(f"spell {spell_id}: listed in spell_disabled")
    row = {
        "class": spec["class"],
        "spell_id": spell_id,
        "name": eff["name"],
        **chain_metadata(spell_id),
        "bucket": bucket,
        "effect_ids": [eff["effect1"], eff["effect2"], eff["effect3"]],
        "aura_ids": [eff["aura1"], eff["aura2"], eff["aura3"]],
        "target_a": [eff["targ_a1"], eff["targ_a2"], eff["targ_a3"]],
        "target_b": [eff["targ_b1"], eff["targ_b2"], eff["targ_b3"]],
        "power_type": eff["power_type"],
        "mana_cost": eff["mana_cost"],
        "range_index": eff["range_index"],
        "cast_time_index": eff["cast_time_index"],
        "duration_index": eff["duration_index"],
        "speed": eff["speed"],
        "stances": eff["stances"],
        "stances_not": eff["stances_not"],
        "totem": [eff["totem1"], eff["totem2"]],
        "reagents": [eff[f"reagent{i}"] for i in range(1, 9)],
        "equipped_item_class": eff["equipped_item_class"],
        "passive": bool(eff["attributes"] & SPELL_ATTR_PASSIVE),
        # Fork `NeedsComboPoints` (SpellEntry.h:958): finishers are
        # rejected at cast time without combo on the target, so the
        # harness must build a point first.
        "needs_combo_points": bool(eff["attributes_ex"] & FINISHING_MOVE_EX),
        "cooldown_ms": eff["recovery_time"],
        "script_name": eff["script_name"],
        "build": eff["build"],
        "expected": spec["expected"],
    }
    if "campaign" in spec:
        row["campaign"] = spec["campaign"]
    return row


def resolve_dbc_dir() -> pathlib.Path:
    raw = os.environ.get("MANGOS_DBC_DIR", "~/azeroth-dev-fixtures/datadir/5875/dbc")
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
    _magic, count, fields, record_size, _string_size = struct.unpack_from("<4sIIII", data, 0)
    rows = []
    for index in range(count):
        offset = 20 + index * record_size
        rows.append(struct.unpack_from("<" + "I" * fields, data, offset))
    return rows


def load_talent_tables(
    dbc_path: pathlib.Path | None = None,
) -> tuple[dict[int, dict], dict[int, int]]:
    dbc_path = dbc_path or resolve_dbc_dir()
    talents = {}
    for row in read_dbc_u32_rows(dbc_path / "Talent.dbc"):
        talents[row[0]] = {
            "tab": row[1],
            "row": row[2],
            "col": row[3],
            "rank_spells": list(row[4:9]),
            "depends_on": row[13],
            "depends_on_rank": row[16],
            "depends_on_spell": row[20],
        }
    tabs = {row[0]: row[12] for row in read_dbc_u32_rows(dbc_path / "TalentTab.dbc")}
    return talents, tabs


#: Fork `CalculateTalentsPoints` at 60 (`Player.cpp:20248`): the seed
#: book must settle at or under this, else `InitTalentForLevel` ->
#: `UpdateFreeTalentPoints(resetIfNeed=true)` -> `ResetTalents` wipes
#: every talent rank spell on login (fork-exact; verified against the
#: vmangos oracle). Paladin live run proved it: a 55-point seed lost
#: 18 spells (CAST_RESULT 145 on 20066/20218 + logout-save deletion).
TALENT_POINTS_FOR_LEVEL_60 = 51


def talent_point_costs(
    dbc_path: pathlib.Path | None = None,
) -> dict[int, int]:
    """Fork `GetTalentSpellCost` (DBCStores.cpp:462): rank + 1 per
    spell in a Talent.dbc RankID list, else 0. Pure/file-only."""
    talents, _tabs = load_talent_tables(dbc_path)
    costs: dict[int, int] = {}
    for talent in talents.values():
        for rank, spell_id in enumerate(talent["rank_spells"]):
            if spell_id:
                costs[spell_id] = rank + 1
    return costs


def settlable_seed(
    seed_ids: set[int],
    audited_ids: set[int],
    costs: dict[int, int],
    max_points: int = TALENT_POINTS_FOR_LEVEL_60,
) -> tuple[list[int], list[int], int]:
    """Drop merged-chain extras no legal level-60 book can hold.

    A merged S7 component can pull a talent-tree rank into a
    trainer-taught chain (Paladin: Holy Shock 20473, Sanctuary 20911,
    Holy Shield 20925, Consecration 26573). Seeding one pushes the
    fork recount over the cap and login wipes the whole talent book.
    Only ranks outside BOTH audited sets (G1 union, talent known)
    that carry a talent cost are excluded; everything else stays, so
    trainer prev-rank behavior is unchanged. Returns
    (seed, excluded, talent_cost). Pure function.
    """
    excluded = sorted(s for s in seed_ids if s not in audited_ids and costs.get(s, 0) > 0)
    seed = sorted(seed_ids - set(excluded))
    used = sum(costs.get(s, 0) for s in seed)
    return seed, excluded, used


def parse_talent_builds(text: str) -> list[dict]:
    builds = []
    pattern = re.compile(
        r"TalentBuild \{\s+class: Class::(\w+),\s+role: Role::(\w+),"
        r'\s+name: "([^"]+)",\s+points: &\[(.*?)\],\s+\}',
        re.S,
    )
    for match in pattern.finditer(text):
        points = [(int(talent_id), int(rank)) for talent_id, rank in re.findall(r"\((\d+),\s*(\d+)\)", match.group(4))]
        builds.append(
            {
                "class": match.group(1),
                "role": match.group(2),
                "name": match.group(3),
                "points": points,
            }
        )
    return builds


def expand_talent_build(
    build: dict, talents: dict[int, dict], tabs: dict[int, int], spell_exists: set[int] | None = None
) -> dict:
    """Expand one checked-in build and audit fork talent legality.

    Final known spell per talent is its highest allocated RankID
    (fork rank replacement). Dependency closure is reported as
    required allocations/deficits; missing prerequisites are not
    silently added to the build.
    """
    class_name = build["class"]
    class_id = CLASS_IDS.get(class_name)
    raw_points = [(int(talent_id), int(rank)) for talent_id, rank in build["points"]]
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
                issues.append(f"talent {talent_id} tab {tab} is not a {class_name} tab")
        rank_spells = [int(spell) for spell in talent["rank_spells"]]
        nonzero_ranks = [spell for spell in rank_spells if spell]
        if rank < 1 or rank > len(nonzero_ranks):
            issues.append(f"talent {talent_id} allocated rank {rank} outside available ranks {len(nonzero_ranks)}")
            continue
        allocated_spells = rank_spells[:rank]
        if spell_exists is not None:
            for spell_id in allocated_spells:
                if spell_id not in spell_exists:
                    issues.append(f"talent {talent_id} rank spell {spell_id} missing from effective spell_template")
        row = int(talent["row"])
        known_spell_id = allocated_spells[-1]
        known_spell_ids.add(known_spell_id)
        all_rank_spell_ids.update(allocated_spells)
        expanded_points.append(
            {
                "talent_id": talent_id,
                "allocated_rank": rank,
                "tab": tab,
                "row": row,
                "col": int(talent["col"]),
                "rank_spell_ids": allocated_spells,
                "known_spell_id": known_spell_id,
            }
        )

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
            allocated
            for other_id, allocated in allocations.items()
            if other_id in talents and int(talents[other_id]["tab"]) == tab and int(talents[other_id]["row"]) < row
        )
        if lower_points < row * 5:
            issues.append(f"talent {talent_id} row {row} has only {lower_points} lower-row points in tab {tab}")

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
            + ", ".join(f"{tid} needs +{deficit}" for tid, deficit in dependency_deficits.items())
        )

    required_spell_ids = sorted(
        {
            int(talents[talent_id]["depends_on_spell"])
            for talent_id in required_allocations
            if talent_id in talents and talents[talent_id].get("depends_on_spell")
        }
    )
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
            issues.append(f"prerequisite talent {talent_id} required rank {rank} outside available ranks")

    return {
        "class": class_name,
        "role": build["role"],
        "name": build["name"],
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
                if build["class"] == class_name and build["role"] == role and build["legal_51"]:
                    chosen = build
                    break
            if chosen is not None:
                break
        selected[class_name] = (
            None
            if chosen is None
            else {
                "name": chosen["name"],
                "role": chosen["role"],
                "total_points": chosen["total_points"],
                "known_spell_ids": chosen["known_spell_ids"],
                "required_spell_ids": chosen["required_spell_ids"],
            }
        )
    return selected


def effective_spell_ids() -> set[int]:
    rows = _query_rows(
        "SELECT entry FROM spell_template t1 WHERE build="
        "(SELECT max(build) FROM spell_template t2 "
        f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})"
    )
    return {int(row[0]) for row in rows if row}


def generate_talent_builds() -> dict:
    builds = parse_talent_builds(TALENT_BUILDS_RS.read_text())
    if len(builds) != 26:
        raise AssertionError(f"expected 26 talent builds, parsed {len(builds)}")
    talents, tabs = load_talent_tables()
    spell_ids = effective_spell_ids()
    expanded = [expand_talent_build(build, talents, tabs, spell_ids) for build in builds]
    selected = select_talent_builds(expanded)
    unresolved = [name for name, build in selected.items() if build is None]
    return {
        "source": str(TALENT_BUILDS_RS),
        "dbc_dir": str(resolve_dbc_dir()),
        "build_count": len(expanded),
        "selected": selected,
        "legal_classes": [name for name, build in selected.items() if build is not None],
        "unresolved_classes": unresolved,
        "builds": expanded,
    }


def talent_build_summary(doc: dict) -> str:
    unresolved = doc.get("unresolved_classes", [])
    if unresolved:
        return (
            "enumerated 26 checked-in builds; legal selected builds "
            f"for {len(doc.get('legal_classes', []))}/9 classes; "
            f"unresolved classes: {', '.join(unresolved)} "
            "(checked-in builds fail fork DependsOn/tier legality); "
            "fan-out blocked"
        )
    return "resolved: legal selected 51-point build for every class"


def load_skill_race_class_info(
    dbc_path: pathlib.Path | None = None,
) -> tuple[dict[int, list[dict]], dict]:
    dbc_path = dbc_path or resolve_dbc_dir()
    skill_line_ids = {row[0] for row in read_dbc_u32_rows(dbc_path / "SkillLine.dbc")}
    raw_rows = read_dbc_u32_rows(dbc_path / "SkillRaceClassInfo.dbc")
    by_skill: dict[int, list[dict]] = {}
    for row in raw_rows:
        skill_id = row[1]
        if skill_id not in skill_line_ids:
            continue
        by_skill.setdefault(skill_id, []).append(
            {
                "skill_id": skill_id,
                "race_mask": row[2],
                "class_mask": row[3],
                "flags": row[4],
                "req_level": row[5],
                "skill_tier_id": row[6],
            }
        )
    return by_skill, {
        "raw_rows": len(raw_rows),
        "filtered_rows": sum(len(rows) for rows in by_skill.values()),
        "skill_line_rows": len(skill_line_ids),
    }


def spell_fit_by_class_and_race(
    spell_id: int,
    race: int,
    class_id: int,
    level: int,
    sla_by_spell: dict[int, list[dict]],
    rc_by_skill: dict[int, list[dict]],
) -> bool:
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


def _load_pre_g1() -> dict:
    """Shared S2 train-set inputs: effective rows, SLA rows, talent
    spells, SkillRaceClassInfo, and playable races per class."""
    effective = {
        int(row[0]): {"spell_level": int(row[1]), "family": int(row[2])}
        for row in _query_rows(
            "SELECT entry,spellLevel,spellFamilyName "
            "FROM spell_template t1 WHERE build="
            "(SELECT max(build) FROM spell_template t2 "
            f"WHERE t1.entry=t2.entry AND build <= {SUPPORTED_BUILD})"
        )
        if row
    }
    sla_rows = []
    sla_by_spell: dict[int, list[dict]] = {}
    for row in _query_rows(
        "SELECT skill_id,spell_id,race_mask,class_mask,"
        "superseded_by_spell "
        f"FROM skill_line_ability WHERE build={SUPPORTED_BUILD} "
        "ORDER BY id"
    ):
        ability = {
            "skill_id": int(row[0]),
            "spell_id": int(row[1]),
            "race_mask": int(row[2]),
            "class_mask": int(row[3]),
            "superseded_by_spell": int(row[4] or 0),
        }
        sla_rows.append(ability)
        sla_by_spell.setdefault(ability["spell_id"], []).append(ability)

    talents, _tabs = load_talent_tables()
    talent_spell_ids = {int(spell) for talent in talents.values() for spell in talent["rank_spells"] if spell}
    rc_by_skill, rc_counts = load_skill_race_class_info()
    races_by_class: dict[int, list[int]] = {}
    for row in _query_rows("SELECT race,class FROM playercreateinfo ORDER BY race,class"):
        races_by_class.setdefault(int(row[1]), []).append(int(row[0]))
    return {
        "effective": effective,
        "sla_rows": sla_rows,
        "sla_by_spell": sla_by_spell,
        "talent_spell_ids": talent_spell_ids,
        "rc_by_skill": rc_by_skill,
        "rc_counts": rc_counts,
        "races_by_class": races_by_class,
    }


def pre_g1_ids_for_class(
    class_name: str,
    pre_g1: dict | None = None,
) -> set[int]:
    """Exact G1 trainable union for one class (S2 gate, pre-race)."""
    pre_g1 = pre_g1 if pre_g1 is not None else _load_pre_g1()
    class_id = CLASS_IDS[class_name]
    class_bit = 1 << (class_id - 1)
    effective = pre_g1["effective"]
    talent_spell_ids = pre_g1["talent_spell_ids"]
    union: set[int] = set()
    for ability in pre_g1["sla_rows"]:
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
        union.add(spell_id)
    return union


def generate_g1_audit() -> dict:
    pre_g1 = _load_pre_g1()
    effective = pre_g1["effective"]
    sla_rows = pre_g1["sla_rows"]
    sla_by_spell = pre_g1["sla_by_spell"]
    talent_spell_ids = pre_g1["talent_spell_ids"]
    rc_by_skill = pre_g1["rc_by_skill"]
    rc_counts = pre_g1["rc_counts"]
    races_by_class = pre_g1["races_by_class"]

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
                spell_id
                for spell_id in pre_g1
                if spell_fit_by_class_and_race(spell_id, race, class_id, 60, sla_by_spell, rc_by_skill)
            }
            per_race[str(race)] = len(fit)
            post_g1.update(fit)
        classes[class_name] = {
            "pre_g1_count": len(pre_g1),
            "post_g1_count": len(post_g1),
            "excluded_spell_ids": sorted(pre_g1 - post_g1),
            "per_race_counts": per_race,
        }
    unresolved = [name for name, data in classes.items() if data["excluded_spell_ids"]]
    return {
        "level": 60,
        "skill_race_class_info": rc_counts,
        "classes": classes,
        "unresolved_classes": unresolved,
        "resolved": not unresolved,
    }


def merged_components(spell_ids: set[int], pre_g1: dict | None = None) -> list[dict]:
    """S7 rank components: shipped `spell_chain` groups joined by
    `skill_line_ability.superseded_by_spell` forward links (DB wins on
    conflict by assertion in validate_row's chain_metadata).

    The cast spell per component is max (spell_level, spell_id): the
    level-60-readable last rank. Returns one dict per component with
    sorted members, the shipped chain_firsts it merged, and the cast
    spell id.
    """
    pre_g1 = pre_g1 if pre_g1 is not None else _load_pre_g1()
    effective = pre_g1["effective"]
    members = sorted(spell_ids)
    first_of = {spell: chain_metadata(spell)["chain_first"] for spell in members}
    parent = {spell: spell for spell in members}

    def find(a: int) -> int:
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def union(a: int, b: int) -> None:
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[ra] = rb

    by_first: dict[int, list[int]] = {}
    for spell in members:
        by_first.setdefault(first_of[spell], []).append(spell)
    for group in by_first.values():
        for other in group[1:]:
            union(group[0], other)
    in_set = set(members)
    for ability in pre_g1["sla_rows"]:
        source = ability["spell_id"]
        target = ability["superseded_by_spell"]
        if target and source in in_set and target in in_set:
            union(source, target)
    comps: dict[int, list[int]] = {}
    for spell in members:
        comps.setdefault(find(spell), []).append(spell)
    result = []
    for group in comps.values():
        cast = max(group, key=lambda s: (effective[s]["spell_level"], s))
        result.append(
            {
                "members": sorted(group),
                "chain_firsts": sorted({first_of[s] for s in group}),
                "cast_spell_id": cast,
            }
        )
    result.sort(key=lambda comp: comp["cast_spell_id"])
    return result


def _targets_of(eff: dict) -> set[int]:
    return {eff[f"targ_a{i}"] for i in (1, 2, 3)} | {eff[f"targ_b{i}"] for i in (1, 2, 3)}


def _effects_of(eff: dict) -> set[int]:
    return {eff["effect1"], eff["effect2"], eff["effect3"]}


def derive_bucket(spell_id: int, eff: dict) -> str:
    """First-match bucket per SPELL_MATRIX.md §3. Pure function of the
    effective row (offline-testable)."""
    if spell_id in PALADIN_DUAL_IDS:
        return "dual"
    if eff["attributes"] & SPELL_ATTR_PASSIVE:
        return "passive"
    targets = _targets_of(eff)
    effects = _effects_of(eff)
    auras = {eff["aura1"], eff["aura2"], eff["aura3"]}
    if effects & RESURRECT_EFFECTS:
        return "corpse"
    if MOD_SHAPESHIFT_AURA in auras:
        # An actual shapeshift form (aura 36 is the fork's
        # SPELL_AURA_MOD_SHAPESHIFT). Pilot hand-labels these "form";
        # the fan-out derives the same bucket from the shared owner.
        return "form"
    allow_unshifted = bool(eff.get("attributes_ex2", 0) & ALLOW_WHILE_NOT_SHAPESHIFTED_EX2)
    if (eff["stances"] and not allow_unshifted) or eff["totem1"] or eff["totem2"]:
        # Stances-gated without the unshifted allowance: the fork gate
        # rejects these from caster form (the harness's rest state).
        # StancesNot alone never blocks at form 0, and Moonkin-bit
        # rows *with* the allowance cast from caster form, so neither
        # forces this bucket.
        return "form_totem"
    if (
        any(eff[f"reagent{i}"] for i in range(1, 9))
        or eff["attributes"] & USES_RANGED_SLOT_ATTR
        or eff["equipped_item_class"] != -1
    ):
        return "item_reagent_ammo"
    if targets & PARTY_TARGETS or effects & DUEL_EFFECTS:
        return "party_duel"
    if targets & PET_TARGETS or effects & (PET_SUMMON_EFFECTS | LEARN_PET_EFFECTS):
        return "pet"
    if targets & (LOCATION_TARGETS | AOE_TARGETS) or effects & GROUND_EFFECTS:
        return "ground_area"
    if targets & FRIENDLY_TARGETS:
        return "friendly"
    if targets & HOSTILE_TARGETS:
        return "hostile_dummy"
    return "self"


def max_direct_heal(eff: dict) -> int:
    """Largest raw heal of the row's HEAL effects (base + full dice,
    no level scaling, no bonuses): the observability ceiling. Pure."""
    best = 0
    for i in (1, 2, 3):
        if eff[f"effect{i}"] != 10:  # SPELL_EFFECT_HEAL
            continue
        best = max(best, eff[f"effect_base_points{i}"] + eff[f"effect_die_sides{i}"] * eff[f"effect_base_dice{i}"])
    return best


#: Heals below this raw maximum cannot clear regen noise on the wire
#: (one discrete regen tick is ~34); they ride as marker-only, never
#: as heal legs that could only FAIL.
SUB_OBSERVABLE_HEAL_MAX = 10


def _verify_aura(spell_id: int, eff: dict) -> int:
    """Aura id to observe: the applied aura when it is a separate
    spell (DoT pattern: max aura id above the cast id), else the cast
    spell id itself (its presence in fields 47-94)."""
    nonzero = [a for a in (eff["aura1"], eff["aura2"], eff["aura3"]) if a]
    best = max(nonzero) if nonzero else 0
    return best if best > spell_id else spell_id


def derive_expected(spell_id: int, eff: dict, bucket: str) -> dict:
    """Observable outcome for one cast row. SCRIPT_EFFECT rows keep
    their intent kind but ride as classify_only (G4 script evidence
    unknown); threat-only and dispel-only rows are marker-only."""
    effects = _effects_of(eff)
    auras = {eff["aura1"], eff["aura2"], eff["aura3"]}
    script = eff["script_name"]
    has_aura = any(auras)
    if bucket == "dual":
        return {
            "kind": "dual_outcome",
            "legs": [
                {"bucket": "friendly", "kind": "heal_delta_target"},
                {"bucket": "hostile_dummy", "kind": "damage_log", "opcode": 592},
            ],
            "note": "T25 TARGET_UNIT(25): needs both the friendly heal "
            "leg and the hostile damage leg plus the facing FAIL leg; "
            "no single-bucket home until both legs pass",
        }
    if bucket == "passive":
        return {"kind": "learned_spell"}
    if bucket == "corpse":
        return {"kind": "resurrect_request"}
    if MOD_SHAPESHIFT_AURA in auras:
        # Entering the form is verified by its aura on the caster (the
        # pilot Bear/Cat pattern). Placed before the SCRIPT branch:
        # form rows carry Effect 77, whose behavior lives in the form
        # aura itself, not in an indexed script body.
        return {"kind": "form_change"}
    if (
        effects & THREAT_EFFECTS
        and not has_aura
        and not effects & (HEAL_EFFECTS | SCHOOL_DAMAGE_EFFECTS | SCRIPT_EFFECTS | APPLY_AURA_EFFECTS)
    ):
        return {"kind": "none_marker", "note": "threat-only: no wire-observable outcome; classify_only"}
    if (
        effects & DISPEL_EFFECTS
        and not has_aura
        and not effects & (HEAL_EFFECTS | SCHOOL_DAMAGE_EFFECTS | APPLY_AURA_EFFECTS)
    ):
        return {"kind": "none_marker", "note": "dispel needs a debuffed ally; classify_only"}
    if SCRIPT_EFFECTS & effects:
        if HEAL_EFFECTS & effects or "Holy Light" in eff["name"] or "Flash of Light" in eff["name"]:
            kind = "heal_delta_target"
        else:
            kind = "damage_log"
        expected: dict = {"kind": kind}
        if kind == "damage_log":
            expected["opcode"] = 592
        expected["note"] = (
            "Effect SCRIPT_EFFECT: behavior lives in spell scripts, "
            "not SpellEffects; classify_only until G4 script evidence "
            "(script-evidence-unknown)"
        )
        expected["classify_only"] = "script"
        return expected
    if script == "spell_paladin_judgement_of_command_dummy" and not (
        has_aura or effects & (HEAL_EFFECTS | SCHOOL_DAMAGE_EFFECTS)
    ):
        return {
            "kind": "damage_log",
            "opcode": 592,
            "note": "damage via the Judgement of Command dummy-body "
            "script chain; classify_only until that script body is "
            "indexed (script-evidence-unknown)",
            "classify_only": "script",
        }
    if HEAL_EFFECTS & effects and 67 not in effects:
        ceiling = max_direct_heal(eff)
        if ceiling < SUB_OBSERVABLE_HEAL_MAX:
            return {
                "kind": "none_marker",
                "note": f"sub-observable heal (raw max {ceiling} < "
                f"{SUB_OBSERVABLE_HEAL_MAX}, below one regen "
                "tick); classify_only",
            }
    if HEAL_EFFECTS & effects:
        return {"kind": "heal_delta_target"}
    if SCHOOL_DAMAGE_EFFECTS & effects:
        return {"kind": "damage_log", "opcode": 592}
    if (
        effects & TELEPORT_EFFECTS
        and not has_aura
        and not effects
        & (HEAL_EFFECTS | SCHOOL_DAMAGE_EFFECTS | APPLY_AURA_EFFECTS | SCRIPT_EFFECTS | DISPEL_EFFECTS | THREAT_EFFECTS)
    ):
        return {
            "kind": "teleport",
            "note": "teleport leaves the bench map; cross-map leg verifies the transfer, never an object",
        }
    if bucket == "ground_area":
        return {"kind": "object_created"}
    if bucket == "item_reagent_ammo":
        reagents = [eff[f"reagent{i}"] for i in range(1, 9) if eff[f"reagent{i}"]]
        if reagents:
            return {"kind": "item_consumed", "reagent": reagents[0]}
        return {
            "kind": "aura_on_caster",
            "aura": _verify_aura(spell_id, eff),
            "note": "equipped-item gate: requires the item equipped; fixture-gear when the starter lacks it",
        }
    if APPLY_AURA_EFFECTS & effects or has_aura:
        aura = _verify_aura(spell_id, eff)
        if bucket == "self":
            return {"kind": "aura_on_caster", "aura": aura}
        if bucket == "form_totem":
            # Stance-gated rows keep their natural target: feral
            # abilities land their aura on the enemy (Growl/Bash/Rip),
            # self-target rows (Enrage/Prowl) on the caster. Bucket
            # alone cannot decide the side; the targets can. 22 is
            # areaselect-enemy around the caster (Demoralizing Roar).
            if _targets_of(eff) & (HOSTILE_TARGETS | FRIENDLY_TARGETS | AOE_TARGETS | {22}):
                return {"kind": "aura_on_target", "aura": aura}
            return {"kind": "aura_on_caster", "aura": aura}
        return {"kind": "aura_on_target", "aura": aura}
    return {"kind": "none_marker", "note": "EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only"}


def g1_summary(doc: dict) -> str:
    counts = ", ".join(f"{name} {data['post_g1_count']}" for name, data in doc.get("classes", {}).items())
    if doc.get("resolved"):
        return f"resolved: generator replay 0 exclusions ({counts})"
    return f"unresolved: generator replay exclusions for {', '.join(doc.get('unresolved_classes', []))} ({counts})"


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
        g1_builds = {"resolved": False, "classes": {}, "artifact": G1_AUDIT_PATH.name}
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
            "selected": {},
            "legal_classes": [],
            "unresolved_classes": list(CLASS_IDS),
            "build_count": 0,
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
        "g4_script_evidence": "caveat: SCRIPT_EFFECT rows ride as BLOCKED script-evidence-unknown",
        "g5_trainer_equivalence": "caveat: bench-vs-production trainer equivalence unverified",
        "rows": rows,
    }


def union_seed_spells(g1_union: set[int], talent_known: set[int]) -> tuple[list[int], list[int]]:
    """Pure seed-union helper (offline-testable): sorted seed input is
    the G1 union plus the selected talent build's known spells; the
    overlap is returned separately so callers assert it explicitly."""
    overlap = sorted(g1_union & talent_known)
    return sorted(g1_union | talent_known), overlap


def selected_talent_known_spells(class_name: str, talent_doc: dict | None = None) -> tuple[str, list[int]]:
    """Selected 51-point build name + known spell ids for one class.

    Reads the checked-in talent-builds artifact when present (no DB),
    else regenerates it (needs DB + DBC). Raises when the class has no
    legal selected build: fan-out is closed for that class.
    """
    if talent_doc is None:
        if TALENT_BUILDS_PATH.exists():
            talent_doc = json.loads(TALENT_BUILDS_PATH.read_text())
        else:
            talent_doc = generate_talent_builds()
    selected = talent_doc.get("selected", {}).get(class_name)
    if not selected:
        raise AssertionError(f"{class_name}: no legal selected 51-point talent build; per-class fan-out is closed")
    return selected["name"], list(selected["known_spell_ids"])


def derive_full_rows(class_name: str = "Paladin", pre_g1: dict | None = None, talent_doc: dict | None = None) -> dict:
    """Per-class fan-out: one cast row per merged S7 component over the
    G1 union plus the selected talent build's known spells, plus the
    all-rank seed list (every cast row's all_rank_ids, so trainer
    prev-rank and SelectAuraRankForLevel behave as on a trained
    character).

    Also audits the bucket derivation over the G1 union against the
    pinned distribution; any drift raises instead of emitting.
    """
    pre_g1 = pre_g1 if pre_g1 is not None else _load_pre_g1()
    union = pre_g1_ids_for_class(class_name, pre_g1)
    build_name, talent_known_ids = selected_talent_known_spells(class_name, talent_doc)
    talent_known = set(talent_known_ids)
    missing = sorted(talent_known - set(pre_g1["effective"]))
    if missing:
        raise AssertionError(
            f"{class_name}: talent build {build_name} known spells missing from effective spell_template: {missing}"
        )
    seed_input, overlap = union_seed_spells(union, talent_known)
    pinned = CLASS_FANOUT_EXPECTATIONS.get(class_name)
    if pinned is not None:
        if len(union) != pinned["union"]:
            raise AssertionError(f"{class_name} G1 union is {len(union)}, want {pinned['union']}")
        if len(talent_known) != pinned["talent_known"]:
            raise AssertionError(f"{class_name} talent known is {len(talent_known)}, want {pinned['talent_known']}")
        if overlap != pinned["overlap"]:
            raise AssertionError(f"{class_name} talent/G1 overlap drifted: {overlap}")
    full_counts: dict[str, int] = {}
    combo_builders: list[int] = []
    for spell_id in sorted(union):
        eff = effective_row(spell_id)
        bucket = derive_bucket(spell_id, eff)
        full_counts[bucket] = full_counts.get(bucket, 0) + 1
        if ADD_COMBO_POINTS_EFFECT in _effects_of(eff):
            combo_builders.append(spell_id)
    # Lowest-rank builder: one combo point for the least dummy
    # damage (a last-rank builder can kill the dummy before the
    # finisher it feeds).
    combo_builder = min(combo_builders) if combo_builders else None
    pinned_buckets = CLASS_FULL_BUCKET_COUNTS.get(class_name)
    if pinned_buckets is not None:
        zero_filled = {bucket: full_counts.get(bucket, 0) for bucket in pinned_buckets}
        if zero_filled != pinned_buckets:
            raise AssertionError(f"{class_name} bucket derivation drifted: {zero_filled} != {pinned_buckets}")
    comps = merged_components(set(seed_input), pre_g1)
    cast_ids = [comp["cast_spell_id"] for comp in comps]
    shipped_lasts = set()
    for spell_id in seed_input:
        shipped_lasts.add(chain_metadata(spell_id)["last_rank_id"])
    if not set(cast_ids) <= shipped_lasts:
        raise AssertionError(
            f"merged cast set is not a subset of shipped lasts: {sorted(set(cast_ids) - shipped_lasts)}"
        )
    if 21563 in cast_ids:
        raise AssertionError("spell_disabled 21563 reached the cast set")
    rows = []
    for comp in comps:
        cast = comp["cast_spell_id"]
        eff = effective_row(cast)
        bucket = derive_bucket(cast, eff)
        expected = derive_expected(cast, eff, bucket)
        spec: dict = {"class": class_name, "bucket": bucket, "expected": expected}
        row = validate_row(cast, spec, eff)
        row["merged_chain"] = {
            "members": comp["members"],
            "chain_firsts": comp["chain_firsts"],
        }
        if "classify_only" in expected:
            row["campaign"] = f"classify_only_{expected['classify_only']}"
        elif expected["kind"] in ("none_marker", "dual_outcome"):
            row["campaign"] = "classify_only"
        rows.append(row)
    rows.sort(key=lambda r: (r["class"], r["spell_id"]))
    raw_seed = set(seed_input) | {rank for row in rows for rank in row["all_rank_ids"]}
    if not set(seed_input) <= raw_seed:
        raise AssertionError("seed lost input spells")
    # Settlable book: merged extras no legal level-60 book holds are
    # excluded before writing, else fork login wipes the talent book
    # (Paladin 55-point seed lost 18 spells live).
    seed, excluded, used = settlable_seed(raw_seed, set(seed_input), talent_point_costs())
    if used > TALENT_POINTS_FOR_LEVEL_60:
        raise AssertionError(
            f"{class_name}: seed talent cost {used} exceeds {TALENT_POINTS_FOR_LEVEL_60}; no legal book holds it"
        )
    for row in rows:
        if row["spell_id"] in excluded:
            raise AssertionError(f"{class_name}: cast spell {row['spell_id']} excluded from its own seed")
        if row["spell_id"] not in seed:
            raise AssertionError(
                f"{class_name}: cast spell {row['spell_id']} not seeded; the cast would fail CAST_RESULT(145)"
            )
    if pinned is not None:
        if sorted(excluded) != pinned["excluded"]:
            raise AssertionError(f"{class_name} seed exclusions drifted: {sorted(excluded)}")
    cast_buckets: dict[str, int] = {}
    for row in rows:
        cast_buckets[row["bucket"]] = cast_buckets.get(row["bucket"], 0) + 1
    return {
        "rows": rows,
        "seed_spell_ids": seed,
        "seed_excluded_spell_ids": excluded,
        "seed_talent_cost": used,
        "cast_count": len(rows),
        "shipped_chain_count": len(shipped_lasts),
        "merged_chain_count": len(comps),
        "full_counts": full_counts,
        "cast_bucket_counts": cast_buckets,
        "g1_union_count": len(union),
        "talent_build_name": build_name,
        "talent_known_count": len(talent_known),
        "talent_known_spell_ids": sorted(talent_known),
        "talent_g1_overlap": overlap,
        "combo_builder_spell": combo_builder,
    }


def generate_full(class_name: str = "Paladin") -> dict:
    """Per-class doc: derived cast rows for one class only, mode=full.

    Pilot rows stay in MATRIX_PATH; this doc goes to the per-class
    artifact and carries the same g1/g2/g3/g4/g5 context lines.
    """
    pilot_doc = generate_pilot()
    full = derive_full_rows(class_name)
    rows = full["rows"]
    by_class: dict[str, int] = {}
    for row in rows:
        by_class[row["class"]] = by_class.get(row["class"], 0) + 1
    return {
        "mode": "full",
        "supported_build": SUPPORTED_BUILD,
        "per_class_counts": by_class,
        "g1_skillraceclassinfo": pilot_doc["g1_skillraceclassinfo"],
        "g1_audit": pilot_doc["g1_audit"],
        "g2_talent_builds": pilot_doc["g2_talent_builds"],
        "talent_builds": pilot_doc["talent_builds"],
        "g3_isspellvalid": pilot_doc["g3_isspellvalid"],
        "g4_script_evidence": pilot_doc["g4_script_evidence"],
        "g5_trainer_equivalence": pilot_doc["g5_trainer_equivalence"],
        "full_class": class_name,
        "full_seed_spell_ids": {class_name: full["seed_spell_ids"]},
        "full_seed_excluded_spell_ids": {class_name: full["seed_excluded_spell_ids"]},
        "full_seed_talent_cost": full["seed_talent_cost"],
        "full_cast_count": full["cast_count"],
        "full_shipped_chains": full["shipped_chain_count"],
        "full_merged_chains": full["merged_chain_count"],
        "full_bucket_counts": full["full_counts"],
        "full_cast_bucket_counts": full["cast_bucket_counts"],
        "full_g1_union": full["g1_union_count"],
        "full_talent_build": full["talent_build_name"],
        "full_talent_known": full["talent_known_count"],
        "full_talent_known_spell_ids": full["talent_known_spell_ids"],
        "full_combo_builder_spell": full["combo_builder_spell"],
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
        {"spell_id": 145, "prev_spell": 143, "first_spell": 133, "rank": 3, "req_spell": 0},
        {"spell_id": 133, "prev_spell": 0, "first_spell": 133, "rank": 1, "req_spell": 0},
        {"spell_id": 143, "prev_spell": 133, "first_spell": 133, "rank": 2, "req_spell": 0},
    ]
    meta = derive_chain_metadata(synthetic, 143)
    assert meta == {
        "chain_first": 133,
        "rank": 2,
        "all_rank_ids": [133, 143, 145],
        "last_rank_id": 145,
        "rank_count": 3,
        "is_last_rank": False,
    }, meta
    assert derive_chain_metadata([], 6795) == {
        "chain_first": 6795,
        "rank": 1,
        "all_rank_ids": [6795],
        "last_rank_id": 6795,
        "rank_count": 1,
        "is_last_rank": True,
    }
    try:
        derive_chain_metadata(
            [
                {"spell_id": 133, "prev_spell": 0, "first_spell": 133, "rank": 1, "req_spell": 0},
                {"spell_id": 143, "prev_spell": 999, "first_spell": 133, "rank": 2, "req_spell": 0},
            ],
            143,
        )
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
            for key in (
                "class",
                "spell_id",
                "name",
                "bucket",
                "expected",
                "chain_first",
                "rank",
                "all_rank_ids",
                "last_rank_id",
                "rank_count",
                "is_last_rank",
                "effect_ids",
                "aura_ids",
                "target_a",
                "target_b",
                "power_type",
                "mana_cost",
                "range_index",
                "cast_time_index",
                "duration_index",
                "speed",
                "stances",
                "stances_not",
                "totem",
                "reagents",
                "equipped_item_class",
                "passive",
                "script_name",
                "build",
            ):
                assert key in row, (row.get("spell_id"), key)
            assert row["name"], row
            assert row["bucket"] in VALID_BUCKETS, row
            assert row["expected"]["kind"] in VALID_EXPECTED, row
            assert row["all_rank_ids"][row["rank"] - 1] == row["spell_id"], row
            assert row["chain_first"] == row["all_rank_ids"][0], row
            assert row["last_rank_id"] == row["all_rank_ids"][-1], row
            assert row["rank_count"] == len(row["all_rank_ids"]), row
            assert row["is_last_rank"] is (row["spell_id"] == row["last_rank_id"]), row
            assert (row["class"], row["spell_id"]) not in seen, row
            seen.add((row["class"], row["spell_id"]))
    # Pilot artifact, if present, must carry exactly the pinned rows.
    if MATRIX_PATH.exists():
        pilot_doc = json.loads(MATRIX_PATH.read_text())
        if pilot_doc.get("mode") == "pilot":
            assert sorted(r["spell_id"] for r in pilot_doc["rows"]) == sorted(PILOT_ROWS), (
                "pilot rows drifted from PILOT_ROWS"
            )
    # Per-class artifact, if present, must obey the fan-out schema:
    # one class only, closed enums, rank continuity, classify_only
    # tagging exactly on marker/dual/script rows, and consistent
    # recorded counts (cast/seed/g1/talent) with no lost inputs.
    for class_name in CLASS_IDS:
        class_path = class_matrix_path(class_name)
        if not class_path.exists():
            continue
        full_doc = json.loads(class_path.read_text())
        assert full_doc["mode"] == "full", full_doc.get("mode")
        assert full_doc["full_class"] == class_name
        full_rows = full_doc["rows"]
        assert all(r["class"] == class_name for r in full_rows)
        assert full_doc["full_cast_count"] == len(full_rows)
        assert full_doc["full_merged_chains"] == len(full_rows)
        seed_set = set(full_doc["full_seed_spell_ids"][class_name])
        excluded_set = set(full_doc.get("full_seed_excluded_spell_ids", {}).get(class_name, []))
        assert not (seed_set & excluded_set), excluded_set & seed_set
        # Settlable book (fork InitTalentForLevel parity): recompute
        # the talent cost from Talent.dbc and require the recorded
        # cost to match and settle at or under the level-60 cap. An
        # unsettlable seed loses the whole talent book on login
        # (Paladin 55-point seed: CAST_RESULT 145 + save deletion).
        dbc_dir = resolve_dbc_dir()
        if (dbc_dir / "Talent.dbc").exists():
            costs = talent_point_costs(dbc_dir)
            recomputed = sum(costs.get(s, 0) for s in seed_set)
            assert full_doc["full_seed_talent_cost"] == recomputed, (full_doc["full_seed_talent_cost"], recomputed)
            assert recomputed <= TALENT_POINTS_FOR_LEVEL_60, recomputed
            assert excluded_set == {s for s in excluded_set if costs.get(s, 0) > 0}, excluded_set
        else:
            print("SELFTEST Talent.dbc absent; skipping cost recompute")
        for row in full_rows:
            assert row["spell_id"] in seed_set, row["spell_id"]
            assert row["spell_id"] not in excluded_set, row["spell_id"]
            for rank_id in row["all_rank_ids"]:
                assert rank_id in seed_set or rank_id in excluded_set, (row["spell_id"], rank_id)
            assert isinstance(row.get("cooldown_ms"), int), row
            assert isinstance(row.get("needs_combo_points"), bool), row
            assert row["bucket"] in VALID_BUCKETS, row
            assert row["expected"]["kind"] in VALID_EXPECTED, row
            assert row["all_rank_ids"][row["rank"] - 1] == row["spell_id"]
            assert row["chain_first"] == row["all_rank_ids"][0]
            assert "merged_chain" in row and row["merged_chain"]["members"]
            assert row["spell_id"] in row["merged_chain"]["members"]
            tag = row.get("campaign", "")
            needs_tag = row["expected"]["kind"] in ("none_marker", "dual_outcome")
            has_script = row["expected"].get("classify_only") == "script"
            if needs_tag or has_script:
                assert tag.startswith("classify_only"), row
            else:
                assert "campaign" not in row, row
        assert seed_set >= set(full_doc["full_talent_known_spell_ids"])
        assert full_doc["full_talent_known"] == len(full_doc["full_talent_known_spell_ids"])
        assert sum(full_doc["full_cast_bucket_counts"].values()) == len(full_rows)
        if class_name == "Druid":
            # Druid-correctness over the real artifact: form rows are
            # shapeshift entries (aura 36) with form_change outcomes,
            # corpse rows keep resurrect requests, and no row invents
            # expect_cast_failure (the pilot wrong-form probe's kind).
            form_rows = [r for r in full_rows if r["bucket"] == "form"]
            assert form_rows, "Druid artifact has no form rows"
            for row in form_rows:
                assert 36 in row["aura_ids"], row["spell_id"]
                assert row["expected"]["kind"] == "form_change", row
            corpse_rows = [r for r in full_rows if r["bucket"] == "corpse"]
            assert corpse_rows, "Druid artifact has no corpse rows"
            for row in corpse_rows:
                assert row["expected"]["kind"] == "resurrect_request", row
            assert not [r for r in full_rows if r["expected"]["kind"] == "expect_cast_failure"]
            assert excluded_set == {5570, 16857}, excluded_set
        print(f"SELFTEST per-class artifact {class_name} cast={len(full_rows)} seed={len(seed_set)} ok")
    # G1 artifact, if generated, must reproduce the derived
    # matrix's exact level-60 union counts with no exclusions.
    if G1_AUDIT_PATH.exists():
        g1_doc = json.loads(G1_AUDIT_PATH.read_text())
        expected_g1 = {
            "Warrior": 121,
            "Paladin": 160,
            "Hunter": 123,
            "Rogue": 89,
            "Priest": 214,
            "Shaman": 159,
            "Mage": 176,
            "Warlock": 202,
            "Druid": 187,
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
        100: [{"skill_id": 10, "spell_id": 100, "race_mask": 0, "class_mask": 1}],
        101: [{"skill_id": 10, "spell_id": 101, "race_mask": 0, "class_mask": 1}],
    }
    synthetic_rc = {10: [{"race_mask": 1, "class_mask": 1, "flags": 0, "req_level": 40}]}
    assert spell_fit_by_class_and_race(999, 1, 1, 60, synthetic_sla, synthetic_rc) is True
    assert spell_fit_by_class_and_race(100, 1, 1, 60, synthetic_sla, synthetic_rc) is True
    assert spell_fit_by_class_and_race(100, 1, 1, 39, synthetic_sla, synthetic_rc) is False
    assert spell_fit_by_class_and_race(100, 2, 1, 60, synthetic_sla, synthetic_rc) is True
    synthetic_rc[10][0]["flags"] = 0x100
    assert spell_fit_by_class_and_race(100, 1, 1, 60, synthetic_sla, synthetic_rc) is False
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
    assert parsed == [
        {
            "class": "Warrior",
            "role": "MeleeDps",
            "name": "synthetic",
            "points": [(1, 2), (2, 1)],
        }
    ]
    synthetic_talents = {
        1: {
            "tab": 10,
            "row": 0,
            "col": 0,
            "rank_spells": [101, 102, 0, 0, 0],
            "depends_on": 0,
            "depends_on_rank": 0,
            "depends_on_spell": 0,
        },
        2: {
            "tab": 10,
            "row": 1,
            "col": 0,
            "rank_spells": [201, 0, 0, 0, 0],
            "depends_on": 1,
            "depends_on_rank": 1,
            "depends_on_spell": 339,
        },
    }
    synthetic_tabs = {10: 1}
    expanded = expand_talent_build(parsed[0], synthetic_talents, synthetic_tabs, spell_exists={101, 102, 201})
    assert expanded["total_points"] == 3, expanded
    assert expanded["known_spell_ids"] == [102, 201], expanded
    assert expanded["all_rank_spell_ids"] == [101, 102, 201], expanded
    assert expanded["dependency_deficits"] == {}, expanded
    assert expanded["required_spell_ids"] == [339], expanded
    assert expanded["legal_51"] is False, expanded
    missing = expand_talent_build(
        {"class": "Warrior", "role": "MeleeDps", "name": "missing", "points": [(2, 1)]},
        synthetic_talents,
        synthetic_tabs,
        spell_exists={101, 102, 201},
    )
    assert missing["dependency_deficits"] == {"1": 2}, missing
    assert missing["legal_51"] is False and missing["issues"], missing
    duplicate = expand_talent_build(
        {"class": "Warrior", "role": "MeleeDps", "name": "dup", "points": [(1, 1), (1, 1)]},
        synthetic_talents,
        synthetic_tabs,
        spell_exists={101, 102, 201},
    )
    assert any("duplicate" in issue for issue in duplicate["issues"])

    def _eff(**over: object) -> dict:
        base: dict = {
            "attributes": 0x50000,
            "attributes_ex": 0,
            "attributes_ex2": 0,
            "power_type": 0,
            "mana_cost": 0,
            "stances": 0,
            "stances_not": 0,
            "totem1": 0,
            "totem2": 0,
            "equipped_item_class": -1,
            "script_name": "",
            "name": "Synthetic",
        }
        for i in (1, 2, 3):
            base[f"effect{i}"] = 0
            base[f"aura{i}"] = 0
            base[f"targ_a{i}"] = 0
            base[f"targ_b{i}"] = 0
            base[f"effect_base_points{i}"] = 0
            base[f"effect_die_sides{i}"] = 0
            base[f"effect_base_dice{i}"] = 0
        for i in range(1, 9):
            base[f"reagent{i}"] = 0
        base.update(over)
        return base

    # Bucket derivation: first-match wins per SPELL_MATRIX.md §3.
    seal = _eff(effect1=6, aura1=4, targ_a1=1, targ_a2=0)
    assert derive_bucket(20293, seal) == "self"
    assert derive_expected(20293, seal, "self") == {"kind": "aura_on_caster", "aura": 20293}
    blessing = _eff(effect1=6, aura1=99, targ_a1=21)
    assert derive_bucket(19977, blessing) == "friendly"
    assert derive_expected(19977, blessing, "friendly") == {"kind": "aura_on_target", "aura": 19977}
    judgement = _eff(effect1=6, aura1=92, targ_a1=6)
    assert derive_bucket(20184, judgement) == "hostile_dummy"
    assert derive_expected(20184, judgement, "hostile_dummy") == {"kind": "aura_on_target", "aura": 20184}
    consecration = _eff(effect1=27, aura1=3, targ_a1=18)
    assert derive_bucket(20924, consecration) == "ground_area"
    assert derive_expected(20924, consecration, "ground_area") == {"kind": "object_created"}
    greater = _eff(effect1=6, aura1=99, targ_a1=61, reagent1=21177)
    assert derive_bucket(25916, greater) == "item_reagent_ammo"
    assert derive_expected(25916, greater, "item_reagent_ammo") == {"kind": "item_consumed", "reagent": 21177}
    protection = _eff(effect1=6, aura1=39, aura2=25, targ_a1=57, targ_a2=57)
    assert derive_bucket(10278, protection) == "party_duel"
    assert derive_expected(10278, protection, "party_duel") == {"kind": "aura_on_target", "aura": 10278}
    shock = _eff(effect1=3, targ_a1=25, script_name="spell_paladin_holy_shock", name="Holy Shock")
    assert derive_bucket(20930, shock) == "dual"
    assert derive_expected(20930, shock, "dual")["kind"] == "dual_outcome"
    holy_light = _eff(effect1=77, targ_a1=21, name="Holy Light")
    assert derive_bucket(635, holy_light) == "friendly"
    scripted = derive_expected(635, holy_light, "friendly")
    assert scripted["kind"] == "heal_delta_target"
    assert scripted["classify_only"] == "script"
    dispel = _eff(effect1=38, effect2=38, targ_a1=21, targ_a2=21, name="Cleanse")
    assert derive_expected(4987, dispel, "friendly")["kind"] == "none_marker"
    threat = _eff(effect1=63, targ_a1=6, name="Righteous Fury")
    assert derive_expected(25781, threat, "hostile_dummy") == {
        "kind": "none_marker",
        "note": "threat-only: no wire-observable outcome; classify_only",
    }
    passive_eff = _eff(attributes=0x40, effect1=6, aura1=4, targ_a1=1)
    assert derive_bucket(20048, passive_eff) == "passive"
    assert derive_expected(20048, passive_eff, "passive") == {"kind": "learned_spell"}
    equip_only = _eff(effect1=6, aura1=4, targ_a1=1, equipped_item_class=2, name="Seal")
    assert derive_bucket(20164, equip_only) == "item_reagent_ammo"
    assert derive_expected(20164, equip_only, "item_reagent_ammo") == {
        "kind": "aura_on_caster",
        "aura": 20164,
        "note": "equipped-item gate: requires the item equipped; fixture-gear when the starter lacks it",
    }
    dummy_script = _eff(effect1=6, targ_a1=6, name="Judgement", script_name="spell_paladin_judgement_of_command_dummy")
    dummy_expected = derive_expected(20184, dummy_script, "hostile_dummy")
    assert dummy_expected["kind"] == "damage_log"
    assert dummy_expected["classify_only"] == "script"
    # Sub-observable heals ride marker-only, never as heal legs.
    tiny_heal = _eff(
        effect1=10, targ_a1=21, name="Holy Light", effect_base_points1=0, effect_die_sides1=1, effect_base_dice1=1
    )
    assert derive_bucket(19968, tiny_heal) == "friendly"
    assert derive_expected(19968, tiny_heal, "friendly") == {
        "kind": "none_marker",
        "note": "sub-observable heal (raw max 1 < 10, below one regen tick); classify_only",
    }
    big_heal = _eff(
        effect1=10, targ_a1=21, name="Holy Shock", effect_base_points1=364, effect_die_sides1=31, effect_base_dice1=1
    )
    assert derive_expected(25903, big_heal, "friendly") == {"kind": "heal_delta_target"}
    loh = _eff(effect1=67, targ_a1=21, name="Lay on Hands", effect_base_points1=-1)
    assert derive_expected(10310, loh, "friendly") == {"kind": "heal_delta_target"}
    print("SELFTEST bucket derivation cases ok")
    # Per-class pins reproduce the pre-generalization Paladin
    # literals exactly; Druid pins are the audited values above.
    assert CLASS_FANOUT_EXPECTATIONS["Paladin"] == {
        "union": 160,
        "talent_known": 14,
        "overlap": [],
        "excluded": [20473, 20911, 20925, 26573],
    }
    assert CLASS_FULL_BUCKET_COUNTS["Paladin"] == PALADIN_FULL_BUCKET_COUNTS
    assert CLASS_FANOUT_EXPECTATIONS["Druid"]["union"] == 187
    assert CLASS_FANOUT_EXPECTATIONS["Druid"]["excluded"] == [5570, 16857]
    # Druid derivation: forms, stance gates, corpse, HoT/heal. Forms
    # (shapeshift aura 36, even with Effect 77) derive the form bucket
    # with a form_change outcome, never a script classify_only.
    bear = _eff(effect1=6, aura1=36, effect2=77, aura2=77, stances_not=1073741824, name="Dire Bear Form")
    assert derive_bucket(9634, bear) == "form"
    assert derive_expected(9634, bear, "form") == {"kind": "form_change"}
    moonkin = _eff(effect1=6, aura1=36, effect3=64, aura3=64, name="Moonkin Form")
    assert derive_bucket(24858, moonkin) == "form"
    assert derive_expected(24858, moonkin, "form") == {"kind": "form_change"}
    # Moonkin-bit rows *with* the unshifted allowance cast from caster
    # form: hostile/friendly buckets, never form_totem.
    moonfire = _eff(
        effect1=2, targ_a1=6, stances=1073741824, attributes_ex2=ALLOW_WHILE_NOT_SHAPESHIFTED_EX2, name="Moonfire"
    )
    assert derive_bucket(9835, moonfire) == "hostile_dummy"
    assert derive_expected(9835, moonfire, "hostile_dummy") == {"kind": "damage_log", "opcode": 592}
    # Bear-only rows without the allowance stay form_totem: the fork
    # gate rejects them from caster form (the pilot Growl pattern).
    # The aura side follows the targets: Growl's taunt aura lands on
    # the enemy, Enrage's on the caster.
    growl = _eff(effect1=6, aura1=4, targ_a1=6, stances=144, name="Growl")
    assert derive_bucket(6795, growl) == "form_totem"
    assert derive_expected(6795, growl, "form_totem") == {"kind": "aura_on_target", "aura": 6795}
    enrage = _eff(effect1=6, aura1=4, targ_a1=1, stances=144, name="Enrage")
    assert derive_bucket(5229, enrage) == "form_totem"
    assert derive_expected(5229, enrage, "form_totem") == {"kind": "aura_on_caster", "aura": 5229}
    # StancesNot alone never blocks at form 0: caster-form heals and
    # HoTs derive friendly buckets with observable kinds.
    heal = _eff(effect1=10, targ_a1=21, stances_not=1073741824, name="Healing Touch", effect_base_points1=200)
    assert derive_bucket(25297, heal) == "friendly"
    assert derive_expected(25297, heal, "friendly") == {"kind": "heal_delta_target"}
    hot = _eff(effect1=6, aura1=8, targ_a1=21, stances_not=1073741824, name="Rejuvenation")
    assert derive_bucket(25299, hot) == "friendly"
    assert derive_expected(25299, hot, "friendly") == {"kind": "aura_on_target", "aura": 25299}
    # Corpse precedes form/reagent: Rebirth (Effect 113 + Moonkin-bit
    # gate + reagent) classifies corpse with a resurrect request, so
    # the harness never routes it to the item leg.
    rebirth = _eff(effect1=113, targ_a1=17, stances=1073741824, reagent1=17038, name="Rebirth")
    assert derive_bucket(20748, rebirth) == "corpse"
    assert derive_expected(20748, rebirth, "corpse") == {"kind": "resurrect_request"}
    # Teleport-only ground rows never claim an object: Teleport:
    # Moonglade (Effect 5, no aura/damage/heal) derives a teleport
    # kind for the cross-map leg.
    teleport = _eff(effect1=5, name="Teleport: Moonglade")
    assert derive_expected(18960, teleport, "ground_area") == {
        "kind": "teleport",
        "note": "teleport leaves the bench map; cross-map leg verifies the transfer, never an object",
    }
    # No derivation path invents expect_cast_failure: that kind stays
    # exclusive to the hand-pinned pilot wrong-form probe, where the
    # fork stance gate truly applies.
    for _probe in (bear, moonkin, moonfire, growl, heal, hot, rebirth, teleport):
        assert derive_expected(0, _probe, derive_bucket(0, _probe))["kind"] != "expect_cast_failure"
    print("SELFTEST Druid derivation cases ok")
    # Seed-union helper: G1 union plus talent known, overlap explicit.
    seed, overlap = union_seed_spells({1, 2, 3}, {3, 4})
    assert seed == [1, 2, 3, 4], (seed, overlap)
    assert overlap == [3], overlap
    seed, overlap = union_seed_spells({465, 633}, {20048, 20059})
    assert seed == [465, 633, 20048, 20059] and overlap == []
    # Talent-build selection: first legal build in PREFERRED_TALENT_ROLES
    # order wins (MeleeDps before Healer), classes without one stay None.
    fake_expanded = [
        {
            "class": "Paladin",
            "role": "Healer",
            "name": "heal",
            "legal_51": True,
            "total_points": 51,
            "known_spell_ids": [1],
            "required_spell_ids": [],
        },
        {
            "class": "Paladin",
            "role": "MeleeDps",
            "name": "ret",
            "legal_51": True,
            "total_points": 51,
            "known_spell_ids": [2],
            "required_spell_ids": [],
        },
    ]
    picked = select_talent_builds(fake_expanded)
    assert picked["Paladin"]["name"] == "ret", picked["Paladin"]
    assert picked["Warrior"] is None
    print("SELFTEST seed union + talent selection ok")
    # Settlable seed: costly ranks outside both audited sets are
    # excluded; the recorded book settles at or under the cap, and a
    # cast spell is never excluded (the live 145 regression).
    synth_costs = {20473: 1, 20911: 1, 20066: 1, 20154: 0, 20920: 0}
    synth_audited = {20066, 20154, 20920}
    synth_raw = synth_audited | {20473, 20911}
    seed_out, excluded_out, used_out = settlable_seed(synth_raw, synth_audited, synth_costs)
    assert excluded_out == [20473, 20911], excluded_out
    assert seed_out == [20066, 20154, 20920], seed_out
    assert used_out == 1, used_out
    # An over-cap book with no excludable ranks stays over cap: the
    # caller (derive_full_rows) must refuse it, never seed it.
    _s2, _e2, over_used2 = settlable_seed({1, 2}, {1, 2}, {1: 30, 2: 30}, max_points=51)
    assert over_used2 == 60 and _e2 == [], (_e2, over_used2)
    # The Druid shape: a book settling exactly at the cap with no
    # excludable ranks stays whole (empty exclusion list).
    _s3, _e3, at_cap_used = settlable_seed({11, 12}, {11, 12}, {11: 20, 12: 31}, max_points=51)
    assert _s3 == [11, 12] and _e3 == [] and at_cap_used == 51
    print("SELFTEST settlable seed ok")
    # Wire-shape constants the harness depends on.
    assert (0xF130 << 48) | (705 << 24) | 344 == 0xF1300002C1000158
    print("SELFTEST matrix enums + schema + guid math ok")
    print("SPELL-MATRIX-GEN-SELFTEST-PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pilot", action="store_true", help="emit the pinned pilot matrix")
    parser.add_argument(
        "--full", action="store_true", help="whole-class fan-out (one cast row per merged S7 chain on the last rank)"
    )
    parser.add_argument(
        "--class", dest="class_name", default="Paladin", help="class fanned out by --full (default Paladin)"
    )
    parser.add_argument("--g1", action="store_true", help="replay the SkillRaceClassInfo train-mode gate")
    parser.add_argument("--talent-builds", action="store_true", help="enumerate/audit checked-in 51-point builds")
    parser.add_argument("--self-test", action="store_true", help="offline invariant checks, no DB needed")
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
        print(f"TALENT-BUILDS {talent_build_summary(doc)} -> {TALENT_BUILDS_PATH}")
        for class_name, selected in doc["selected"].items():
            print(f"SELECTED {class_name}: {selected['name'] if selected else 'NONE'}")
        return 2 if doc["unresolved_classes"] else 0
    if args.full and not args.pilot:
        doc = generate_full(args.class_name)
        out_path = class_matrix_path(args.class_name)
        out_path.write_text(json.dumps(doc, indent=1) + "\n")
        print(
            f"MATRIX-GEN-OK mode=full class={args.class_name} "
            f"cast_rows={doc['full_cast_count']} "
            f"shipped={doc['full_shipped_chains']} "
            f"merged={doc['full_merged_chains']} "
            f"g1_delta={doc['full_shipped_chains'] - doc['full_merged_chains']} "
            f"g1_union={doc['full_g1_union']} "
            f"talent={doc['full_talent_build']}:{doc['full_talent_known']} "
            f"seed={len(doc['full_seed_spell_ids'][args.class_name])} "
            f"cast_buckets={doc['full_cast_bucket_counts']} "
            f"-> {out_path}"
        )
        return 0
    doc = generate_pilot()
    MATRIX_PATH.write_text(json.dumps(doc, indent=1) + "\n")
    print(
        f"MATRIX-GEN-OK mode=pilot rows={len(doc['rows'])} classes={sorted(doc['per_class_counts'])} -> {MATRIX_PATH}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
