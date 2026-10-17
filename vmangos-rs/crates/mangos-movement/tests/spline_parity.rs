//! Bit-exact parity against the independent Python oracle
//! (`vmangos-rs/scripts/spline_oracle_gen.py`, a from-source
//! reimplementation of vmangos Movement/spline).

use mangos_movement::move_spline::{
    MoveSplineFlag, MoveSplineInitArgs, compute_fall_elevation,
    compute_fall_elevation_with_velocity, compute_fall_time,
};
use mangos_movement::packet_builder::write_monster_move;
use mangos_movement::spline::EvaluationMode;
use mangos_movement::{MoveSpline, Vec3};
use mangos_protocol::buffer::PacketWriter;

#[path = "spline_oracle.rs"]
mod oracle;

use oracle::{CASES, FALL_PROBES};

const US: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn vec3(b: (u32, u32, u32)) -> Vec3 {
    Vec3::new(
        f32::from_bits(b.0),
        f32::from_bits(b.1),
        f32::from_bits(b.2),
    )
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[test]
fn spline_math_and_packets_match_oracle() {
    for case in CASES {
        let args = MoveSplineInitArgs {
            path: case.controls_bits.iter().map(|b| vec3(*b)).collect(),
            velocity: f32::from_bits(case.velocity_bits),
            spline_id: case.spline_id,
            flags: MoveSplineFlag(case.flags),
            ..MoveSplineInitArgs::default()
        };
        let mut ms = MoveSpline::new();
        ms.initialize(&args);
        let s = ms.spline();

        assert_eq!(s.first(), case.first, "{}: first", case.name);
        assert_eq!(s.last(), case.last, "{}: last", case.name);
        assert_eq!(
            s.mode(),
            if case.catmull {
                EvaluationMode::CatmullRom
            } else {
                EvaluationMode::Linear
            },
            "{}: mode",
            case.name
        );
        assert_eq!(s.is_cyclic(), case.cyclic, "{}: cyclic", case.name);

        for (i, expected) in case.storage_bits.iter().enumerate() {
            let p = s.point(i32::try_from(i).unwrap());
            assert_eq!(
                (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()),
                *expected,
                "{}: storage[{i}]",
                case.name
            );
        }

        for (i, expected) in case.lengths.iter().enumerate() {
            assert_eq!(
                s.length_at(i32::try_from(i).unwrap()),
                *expected,
                "{}: lengths[{i}]",
                case.name
            );
        }

        let mut flat = 0;
        for (k, idx) in (case.first..case.last).enumerate() {
            assert_eq!(
                s.seg_length(idx).to_bits(),
                case.seg_lengths_bits[k],
                "{}: seg_length({idx})",
                case.name
            );
            for u in US {
                let e = s.evaluate_percent_at(idx, u);
                assert_eq!(
                    (e.x.to_bits(), e.y.to_bits(), e.z.to_bits()),
                    case.eval_bits[flat],
                    "{}: eval({idx}, {u})",
                    case.name
                );
                let d = s.evaluate_derivative_at(idx, u);
                assert_eq!(
                    (d.x.to_bits(), d.y.to_bits(), d.z.to_bits()),
                    case.deriv_bits[flat],
                    "{}: deriv({idx}, {u})",
                    case.name
                );
                flat += 1;
            }
        }

        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, 80);
        assert_eq!(hex(&w.into_vec()), case.packet_hex, "{}: packet", case.name);
    }
}

#[test]
fn fall_formulas_match_oracle() {
    for &(kind, a_bits, safe, b_bits, expected) in FALL_PROBES {
        let a = f32::from_bits(a_bits);
        let got = match kind {
            0 => compute_fall_time(a, safe),
            1 => compute_fall_elevation(a),
            _ => compute_fall_elevation_with_velocity(a, safe, f32::from_bits(b_bits)),
        };
        assert_eq!(
            got.to_bits(),
            expected,
            "fall probe kind={kind} a={a} safe={safe} b={}",
            f32::from_bits(b_bits)
        );
    }
}
