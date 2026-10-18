//! Spell-math differential tests against the Python
//! oracle (`spell_math_oracle.rs`, generated from the
//! bench DB + DBC files by
//! `vmangos-rs/scripts/spell_math_oracle_gen.py`).

mod spell_math_oracle;

use mangos_spells::SpellRng;
use mangos_spells::entry::{CastTimeEntry, DurationEntry, RangeEntry, SpellDbcTables};
use spell_math_oracle::{CASTER_LEVELS, cases};

/// Deterministic test RNG: irand returns min then max
/// alternately is overkill — tests only assert bounds,
/// so any in-range value works; use min.
struct MinRng;
impl SpellRng for MinRng {
    fn irand(&mut self, min: i32, _max: i32) -> i32 {
        min
    }
    fn frand01(&mut self) -> f32 {
        0.0
    }
}

#[test]
fn spell_math_matches_oracle() {
    for case in cases() {
        let mut tables = SpellDbcTables::default();
        tables.cast_times.insert(
            case.entry.casting_time_index,
            CastTimeEntry {
                cast_time: case.cast_time.0,
                cast_time_per_level: case.cast_time.1,
                min_cast_time: case.cast_time.2,
            },
        );
        tables.durations.insert(
            case.entry.duration_index,
            DurationEntry {
                duration: case.duration.0,
                duration_per_level: case.duration.1,
                max_duration: case.duration.2,
            },
        );
        tables.ranges.insert(
            case.entry.range_index,
            RangeEntry {
                min: case.range.0,
                max: case.range.1,
            },
        );
        let e = &case.entry;
        assert_eq!(
            e.cast_time_ms(&tables),
            case.expected_cast_ms,
            "cast time spell {} ({})",
            e.id,
            e.name
        );
        assert_eq!(
            e.calculate_duration_ms(&tables),
            case.expected_duration_ms,
            "duration spell {} ({})",
            e.id,
            e.name
        );
        assert!(
            (e.range_max(&tables) - case.range.1).abs() < f32::EPSILON,
            "range spell {}",
            e.id
        );
        // Oracle power cost used max power 1000 for the
        // percentage arm.
        assert_eq!(
            e.power_cost(0, 1000, 1000),
            case.expected_power_cost,
            "power cost spell {} ({})",
            e.id,
            e.name
        );
        for (li, level) in CASTER_LEVELS.iter().enumerate() {
            for eff in 0..3 {
                let (lo, hi) = case.value_bounds[li][eff];
                let mut rng = MinRng;
                let v = e.calculate_effect_value(eff, *level, 0, &mut rng);
                assert!(
                    v >= lo - 0.001 && v <= hi + 0.001,
                    "spell {} ({}) eff {eff} level {level}: {v} not in [{lo}, {hi}]",
                    e.id,
                    e.name
                );
                if (hi - lo).abs() < 0.001 {
                    assert!(
                        (v - lo).abs() < 0.001,
                        "spell {} eff {eff} level {level}: exact {v} != {lo}",
                        e.id
                    );
                }
            }
        }
    }
}
