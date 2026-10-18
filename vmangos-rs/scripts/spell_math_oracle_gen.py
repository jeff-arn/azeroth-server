#!/usr/bin/env python3
"""Spell-math goldens from the bench world DB + DBC files.

Independent oracle: reads spell_template rows and the
SpellCastTimes/SpellDuration/SpellRange DBCs directly,
reimplements the deterministic C++ formulas
(CalculateSpellEffectValue bounds, GetCastTime with
rank 0, GetDuration, CalculatePowerCost flat arm) with
per-operation f32 rounding, and emits a Rust fixture.
The Rust suite rebuilds SpellEntry values from the
snapshotted fields and compares its math against these
numbers.

Usage: spell_math_oracle_gen.py <out.rs>
Env: TEST_MYSQL_URL_WORLD, MANGOS_DBC_DIR (dir containing dbc/).
"""

import os
import struct
import subprocess
import sys
import urllib.parse


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


SPELLS = [116, 172, 139, 695, 772, 2050, 2136, 5242, 6546, 6673]
LEVELS = [1, 19, 60]


def db_query(sql):
    u = urllib.parse.urlparse(
        os.environ["TEST_MYSQL_URL_WORLD"].replace("mysql://", "http://", 1)
    )
    cmd = ["mysql", "-h", u.hostname, "-P", str(u.port or 3306), "-u", u.username]
    cmd += ["-N", "-B", "-D", u.path.lstrip("/"), "-e", sql]
    env = dict(os.environ, MYSQL_PWD=u.password or "")
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if proc.returncode != 0:
        raise SystemExit(f"mysql failed: {proc.stderr.strip()}")
    out = proc.stdout
    return [line.split("\t") for line in out.strip().split("\n") if line]


def read_dbc(path, fmt):
    data = open(path, "rb").read()
    assert data[:4] == b"WDBC", path
    _magic, count, fields, recsize, _ssize = struct.unpack_from("<4sIIII", data)
    rows = {}
    off = 20
    for _ in range(count):
        rec = data[off : off + recsize]
        vals = []
        pos = 0
        for ch in fmt:
            if ch in "nixX":
                vals.append(struct.unpack_from("<I", rec, pos)[0])
                pos += 4
            elif ch == "f":
                vals.append(struct.unpack_from("<f", rec, pos)[0])
                pos += 4
            elif ch == "b":
                vals.append(rec[pos])
                pos += 1
            elif ch == "s":
                vals.append(struct.unpack_from("<I", rec, pos)[0])
                pos += 4
            elif ch == "d":
                pos += 4
        rows[vals[0]] = vals
        off += recsize
    return rows


def s32(v):
    return struct.unpack("<i", struct.pack("<I", v & 0xFFFFFFFF))[0]


COLS = [
    "entry", "school", "category", "dispel", "mechanic",
    "attributes", "attributesEx", "attributesEx2", "attributesEx3",
    "attributesEx4", "targets", "castingTimeIndex", "recoveryTime",
    "categoryRecoveryTime", "maxLevel", "baseLevel", "spellLevel",
    "durationIndex", "powerType", "manaCost", "manCostPerLevel",
    "rangeIndex", "speed", "stackAmount",
]
EFF_COLS = []
for group in [
    "effect{0}", "effectDieSides{0}", "effectBaseDice{0}",
    "effectDicePerLevel{0}", "effectRealPointsPerLevel{0}",
    "effectBasePoints{0}", "effectBonusCoefficient{0}",
    "effectMechanic{0}", "effectImplicitTargetA{0}",
    "effectImplicitTargetB{0}", "effectRadiusIndex{0}",
    "effectApplyAuraName{0}", "effectAmplitude{0}",
    "effectMultipleValue{0}", "effectChainTarget{0}",
    "effectMiscValue{0}", "effectTriggerSpell{0}",
    "effectPointsPerComboPoint{0}", "dmgMultiplier{0}",
]:
    EFF_COLS.append([group.format(i) for i in (1, 2, 3)])
TAIL = [
    "spellVisual1", "manaCostPercentage", "startRecoveryCategory",
    "startRecoveryTime", "spellFamilyName", "spellFamilyFlags",
    "maxAffectedTargets", "dmgClass", "preventionType", "name",
]
ALL_COLS = COLS + [c for g in EFF_COLS for c in g] + TAIL


def effect_value_bounds(row, eff, caster_level):
    """CalculateSpellEffectValue with the irand endpoints."""
    max_level = int(row["maxLevel"])
    base_level = int(row["baseLevel"])
    spell_level = int(row["spellLevel"])
    level = caster_level
    if level > max_level and max_level > 0:
        level = max_level
    elif level < base_level:
        level = base_level
    level -= spell_level
    base_dice = int(eff["effectBaseDice"])
    value = float(int(eff["effectBasePoints"]))
    value = f32(value + f32(level * f32(float(eff["effectRealPointsPerLevel"]))))
    random_points = int(
        f32(float(eff["effectDieSides"]) + level * f32(float(eff["effectDicePerLevel"])))
    )
    if random_points in (0, 1):
        value = f32(value + base_dice)
        return value, value
    lo = min(base_dice, random_points)
    hi = max(base_dice, random_points)
    return f32(value + lo), f32(value + hi)


def main():
    out_path = sys.argv[1]
    dbc_dir = os.environ["MANGOS_DBC_DIR"]
    cast_times = read_dbc(os.path.join(dbc_dir, "SpellCastTimes.dbc"), "niii")
    durations = read_dbc(os.path.join(dbc_dir, "SpellDuration.dbc"), "niii")
    ranges = read_dbc(os.path.join(dbc_dir, "SpellRange.dbc"), "nffxxxxxxxxxxxxxxxxxxx")

    sql_cols = ",".join(f"`{c}`" for c in ALL_COLS)
    ids = ",".join(str(s) for s in SPELLS)
    rows = db_query(
        f"SELECT {sql_cols} FROM spell_template WHERE entry IN ({ids}) "
        "AND build=(SELECT MAX(build) FROM spell_template t2 "
        "WHERE t2.entry=spell_template.entry AND build<=5875) ORDER BY entry"
    )

    lines = [
        "#![allow(clippy::unreadable_literal, clippy::excessive_precision, clippy::too_many_lines, clippy::must_use_candidate)]\n// Generated by vmangos-rs/scripts/spell_math_oracle_gen.py.",
        "// Regenerate against the bench DB/DBC; do not edit by hand.",
        "use mangos_spells::entry::{SpellEffectData, SpellEntry};",
        "",
        "pub struct OracleCase {",
        "    pub entry: SpellEntry,",
        "    pub cast_time: (i32, i32, i32),",
        "    pub duration: (i32, i32, i32),",
        "    pub range: (f32, f32),",
        "    pub expected_cast_ms: u32,",
        "    pub expected_duration_ms: i32,",
        "    pub expected_power_cost: u32,",
        "    pub value_bounds: [[(f32, f32); 3]; 3],",
        "}",
        "",
        "pub const CASTER_LEVELS: [u32; 3] = [1, 19, 60];",
        "",
        "pub fn cases() -> Vec<OracleCase> {",
        "    vec![",
    ]
    for raw in rows:
        row = dict(zip(ALL_COLS, raw))
        effs = []
        for i in range(3):
            effs.append({g.format(0)[:-3] if False else g: row[g] for g in [grp[i] for grp in EFF_COLS]})
        # normalize effect dict keys to the un-suffixed names
        norm_effs = []
        for i in range(3):
            d = {}
            for grp in EFF_COLS:
                key = grp[i]
                base = key[:-1]
                d[base] = row[key]
            norm_effs.append(d)
        ct = cast_times.get(int(row["castingTimeIndex"]), [0, 0, 0, 0])
        du = durations.get(int(row["durationIndex"]), [0, 0, 0, 0])
        rg = ranges.get(int(row["rangeIndex"]), [0, 0.0, 0.0])
        cast_ms = s32(ct[1]) + s32(ct[2]) * (-int(row["baseLevel"]))
        cast_ms = max(cast_ms, s32(ct[3]))
        cast_ms = cast_ms if cast_ms > 0 else 0
        dur_ms = s32(du[1])
        dur_ms = -1 if dur_ms == -1 else abs(dur_ms)
        cost = int(row["manaCost"]) + int(row["manCostPerLevel"]) * (-int(row["baseLevel"]))
        if int(row["manaCostPercentage"]):
            cost += 1000 * int(row["manaCostPercentage"]) // 100
        cost = max(cost, 0)
        bounds = []
        for lvl in LEVELS:
            bounds.append(tuple(effect_value_bounds(row, e, lvl) for e in norm_effs))

        def eff_rs(e):
            return (
                "SpellEffectData { "
                f"effect: {e['effect']}, die_sides: {e['effectDieSides']}, "
                f"base_dice: {e['effectBaseDice']}, "
                f"dice_per_level: {float(e['effectDicePerLevel'])}_f32, "
                f"real_points_per_level: {float(e['effectRealPointsPerLevel'])}_f32, "
                f"base_points: {e['effectBasePoints']}, "
                f"bonus_coefficient: {float(e['effectBonusCoefficient'])}_f32, "
                f"mechanic: {e['effectMechanic']}, "
                f"implicit_target_a: {e['effectImplicitTargetA']}, "
                f"implicit_target_b: {e['effectImplicitTargetB']}, "
                f"radius_index: {e['effectRadiusIndex']}, "
                f"apply_aura_name: {e['effectApplyAuraName']}, "
                f"amplitude: {e['effectAmplitude']}, "
                f"multiple_value: {float(e['effectMultipleValue'])}_f32, "
                f"chain_target: {e['effectChainTarget']}, "
                f"misc_value: {e['effectMiscValue']}, "
                f"trigger_spell: {e['effectTriggerSpell']}, "
                f"points_per_combo_point: {float(e['effectPointsPerComboPoint'])}_f32, "
                f"dmg_multiplier: {float(e['dmgMultiplier'])}_f32 }}"
            )

        name = row["name"].replace("\\", "\\\\").replace('"', '\\"')
        lines.append("        OracleCase {")
        lines.append("            entry: SpellEntry {")
        lines.append(f"                id: {row['entry']}, school: {row['school']}, category: {row['category']},")
        lines.append(f"                dispel: {row['dispel']}, mechanic: {row['mechanic']}, attributes: {row['attributes']},")
        lines.append(
            f"                attributes_ex: {row['attributesEx']}, attributes_ex2: {row['attributesEx2']}, "
            f"attributes_ex3: {row['attributesEx3']}, attributes_ex4: {row['attributesEx4']},"
        )
        lines.append(
            f"                targets: {row['targets']}, casting_time_index: {row['castingTimeIndex']}, "
            f"recovery_time: {row['recoveryTime']}, category_recovery_time: {row['categoryRecoveryTime']},"
        )
        lines.append("                proc_flags: 0, proc_chance: 0, proc_charges: 0,")
        lines.append(
            f"                max_level: {row['maxLevel']}, base_level: {row['baseLevel']}, "
            f"spell_level: {row['spellLevel']}, duration_index: {row['durationIndex']},"
        )
        lines.append(
            f"                power_type: {row['powerType']}, mana_cost: {row['manaCost']}, "
            f"mana_cost_per_level: {row['manCostPerLevel']}, mana_per_second: 0, mana_per_second_per_level: 0,"
        )
        lines.append(
            f"                range_index: {row['rangeIndex']}, speed: {float(row['speed'])}_f32, "
            f"stack_amount: {row['stackAmount']},"
        )
        lines.append("                effects: [")
        for e in norm_effs:
            lines.append(f"                    {eff_rs(e)},")
        lines.append("                ],")
        lines.append(
            f"                spell_visual: {row['spellVisual1']}, mana_cost_percentage: {row['manaCostPercentage']}, "
            f"start_recovery_category: {row['startRecoveryCategory']}, start_recovery_time: {row['startRecoveryTime']},"
        )
        lines.append(
            f"                spell_family_name: {row['spellFamilyName']}, spell_family_flags: {row['spellFamilyFlags']}, "
            f"max_affected_targets: {row['maxAffectedTargets']}, dmg_class: {row['dmgClass']}, prevention_type: {row['preventionType']},"
        )
        lines.append(f'                custom: 0, name: "{name}".to_string(),')
        lines.append("            },")
        lines.append(f"            cast_time: ({s32(ct[1])}, {s32(ct[2])}, {s32(ct[3])}),")
        lines.append(f"            duration: ({s32(du[1])}, {s32(du[2])}, {s32(du[3])}),")
        lines.append(f"            range: ({rg[1]}_f32, {rg[2]}_f32),")
        lines.append(f"            expected_cast_ms: {cast_ms},")
        lines.append(f"            expected_duration_ms: {dur_ms},")
        lines.append(f"            expected_power_cost: {cost},")
        b = ", ".join(
            "[" + ", ".join(f"({lo}_f32, {hi}_f32)" for lo, hi in lvl) + "]"
            for lvl in bounds
        )
        lines.append(f"            value_bounds: [{b}],")
        lines.append("        },")
    lines.append("    ]")
    lines.append("}")
    lines.append("")
    open(out_path, "w").write("\n".join(lines))
    print(f"wrote {out_path} with {len(rows)} spells")


if __name__ == "__main__":
    main()
