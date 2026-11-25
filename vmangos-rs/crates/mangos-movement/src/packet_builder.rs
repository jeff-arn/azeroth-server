//! `PacketBuilder` (`vmangos/src/game/Movement/spline/packet_builder.cpp`),
//! byte-exact: `WriteCommonMonsterMovePart`, `WriteMonsterMove`
//! (linear packed-offset path with the `[-ZERO]` nudge and the
//! duration back-patch, Catmull-Rom and cyclic Catmull-Rom paths),
//! and `WriteCreate` for object-update blobs (1.12.1 shape, i.e.
//! `SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_7_1` so the spline id
//! is included).
//!
//! The packet opcode header and packed guid are the caller's job
//! (see `Unit::UpdateSplineMovement`).

use mangos_protocol::buffer::PacketWriter;

use crate::move_spline::{FacingInfo, MoveSpline, MoveSplineFlag};
use crate::spline::{Spline, Vec3};

/// `Movement.MaxPointsPerPacket` default (`mangosd.conf.dist`).
pub const DEFAULT_MAX_POINTS_PER_PACKET: u32 = 80;

/// `MonsterMoveType` facing selector byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum FacingByte {
    Normal = 0,
    FacingSpot = 2,
    FacingTarget = 3,
    FacingAngle = 4,
}

fn write_vec3(data: &mut PacketWriter, v: Vec3) {
    data.write_f32_le(v.x).write_f32_le(v.y).write_f32_le(v.z);
}

/// `PacketBuilder::WriteCommonMonsterMovePart`: first spline point,
/// spline id, facing block, masked flags (with the fake
/// `Enter_Cycle` and `Runmode` bits), and the full duration (the
/// linear path may back-patch it afterwards).
pub fn write_common_monster_move_part(data: &mut PacketWriter, move_spline: &MoveSpline) {
    let mut flags = move_spline.flags();
    let spline = move_spline.spline();

    write_vec3(data, spline.point(spline.first()));
    data.write_u32_le(move_spline.id());

    // The C++ switch matches the masked facing bits exactly:
    // zero or multiple bits fall through to Normal. (Flag/variant
    // mismatches are unreachable through the init-args API; the
    // zero fallbacks keep the packet well-formed where C++ would
    // reinterpret the union.)
    match flags.raw() & MoveSplineFlag::MASK_FINAL_FACING {
        MoveSplineFlag::FINAL_TARGET => {
            data.write_u8(FacingByte::FacingTarget as u8);
            let g = match move_spline.facing() {
                FacingInfo::Target(g) => g,
                _ => 0,
            };
            data.write_u64_le(g);
        }
        MoveSplineFlag::FINAL_ANGLE => {
            data.write_u8(FacingByte::FacingAngle as u8);
            let a = match move_spline.facing() {
                FacingInfo::Angle(a) => a,
                _ => 0.0,
            };
            data.write_f32_le(a);
        }
        MoveSplineFlag::FINAL_POINT => {
            data.write_u8(FacingByte::FacingSpot as u8);
            let p = match move_spline.facing() {
                FacingInfo::Point(p) => p,
                _ => Vec3::ZERO,
            };
            write_vec3(data, p);
        }
        _ => {
            data.write_u8(FacingByte::Normal as u8);
        }
    }

    // Fake Enter_Cycle flag — the client erases the first spline
    // vertex after the first cycle when it is set.
    if move_spline.is_cyclic() {
        flags.0 |= MoveSplineFlag::ENTER_CYCLE;
    } else {
        flags.0 &= !MoveSplineFlag::ENTER_CYCLE;
    }
    // Fake Runmode flag.
    data.write_u32_le(
        flags.raw() & !MoveSplineFlag::MASK_NO_MONSTER_MOVE | MoveSplineFlag::RUNMODE,
    );
    #[allow(clippy::cast_sign_loss)]
    data.write_u32_le(move_spline.duration() as u32);
}

/// `WriteLinearPath`: count, destination, then packed
/// destination-relative midpoints. Returns the last index written
/// when capped at `max_points_per_packet`, else -1.
fn write_linear_path(
    data: &mut PacketWriter,
    spline: &Spline,
    start: u32,
    max_points_per_packet: u32,
) -> i32 {
    #[allow(clippy::cast_sign_loss)]
    let mut last_idx = (spline.point_count() - 3) as u32;
    let mut last_idx_written: i32 = -1;
    if last_idx - start > max_points_per_packet {
        last_idx = start + max_points_per_packet;
        #[allow(clippy::cast_possible_wrap)]
        {
            last_idx_written = last_idx as i32;
        }
    }
    // real_path[i] == spline.point(1 + i)
    #[allow(clippy::cast_possible_wrap)]
    let real_point = |i: u32| spline.point(1 + i as i32);
    let destination = real_point(last_idx);

    data.write_u32_le(last_idx - start + 1);
    write_vec3(data, destination);
    if last_idx > 1 {
        for i in start..last_idx {
            let mut offset = destination - real_point(i);
            // [-ZERO] The client freezes when it gets a zero offset.
            if offset.x.abs() < 0.25 && offset.y.abs() < 0.25 && offset.z.abs() < 0.25 {
                if offset.z < 0.0 {
                    offset.z += 0.51;
                } else {
                    offset.z += 0.26;
                }
            }
            data.write_packed_xyz(offset.x, offset.y, offset.z);
        }
    }
    last_idx_written
}

/// `WriteCatmullRomPath`: count, then raw points from storage
/// index 2 (all controls except the first).
#[allow(clippy::cast_possible_wrap)]
fn write_catmull_rom_path(data: &mut PacketWriter, spline: &Spline) {
    #[allow(clippy::cast_sign_loss)]
    let count = (spline.point_count() - 3) as u32;
    data.write_u32_le(count);
    for i in 0..count as i32 {
        write_vec3(data, spline.point(2 + i));
    }
}

/// `WriteCatmullRomCyclicPath`: count+1, the fake first point
/// (storage index 1), then count points from index 1 again.
#[allow(clippy::cast_possible_wrap)]
fn write_catmull_rom_cyclic_path(data: &mut PacketWriter, spline: &Spline) {
    #[allow(clippy::cast_sign_loss)]
    let count = (spline.point_count() - 3) as u32;
    data.write_u32_le(count + 1);
    write_vec3(data, spline.point(1)); // fake point, client erases it after first cycle
    for i in 0..count as i32 {
        write_vec3(data, spline.point(1 + i));
    }
}

/// `PacketBuilder::WriteMonsterMove(move_spline, data, firstPoint)`.
/// Returns the last node written (store it via
/// [`MoveSpline::set_last_point_sent`]; the next continuation call
/// passes that + 1 as `first_point`), or -1 when the whole path
/// was written.
#[must_use]
pub fn write_monster_move(
    data: &mut PacketWriter,
    move_spline: &MoveSpline,
    first_point: i32,
    max_points_per_packet: u32,
) -> i32 {
    write_common_monster_move_part(data, move_spline);
    let duration_pos = data.len() - 4;

    let spline = move_spline.spline();
    if move_spline.flags().raw() & MoveSplineFlag::MASK_CATMULL_ROM != 0 {
        if move_spline.flags().cyclic() {
            write_catmull_rom_cyclic_path(data, spline);
        } else {
            write_catmull_rom_path(data, spline);
        }
        -1
    } else {
        #[allow(clippy::cast_sign_loss)]
        let last_node = write_linear_path(data, spline, first_point as u32, max_points_per_packet);
        let duration = move_spline.duration_between(
            0,
            if last_node >= 0 {
                last_node
            } else {
                move_spline.count_spline_points()
            },
        );
        #[allow(clippy::cast_sign_loss)]
        data.put_u32(duration_pos, (duration - move_spline.time_passed()) as u32);
        last_node
    }
}

/// `PacketBuilder::WriteCreate` — the movement block inside
/// `SMSG_UPDATE_OBJECT` create values. No-op for an uninitialized
/// spline, as in C++.
pub fn write_create(data: &mut PacketWriter, move_spline: &MoveSpline) {
    if !move_spline.initialized() {
        return;
    }
    let flags = move_spline.flags();
    data.write_u32_le(flags.raw());

    if flags.final_angle() {
        if let FacingInfo::Angle(a) = move_spline.facing() {
            data.write_f32_le(a);
        }
    } else if flags.final_target() {
        if let FacingInfo::Target(g) = move_spline.facing() {
            data.write_u64_le(g);
        }
    } else if flags.final_point() {
        if let FacingInfo::Point(p) = move_spline.facing() {
            write_vec3(data, p);
        }
    }

    #[allow(clippy::cast_sign_loss)]
    data.write_u32_le(move_spline.time_passed() as u32);
    #[allow(clippy::cast_sign_loss)]
    data.write_u32_le(move_spline.duration() as u32);
    data.write_u32_le(move_spline.id());

    // getPath() is the full storage incl. virtual end points.
    let path = move_spline.spline().points();
    #[allow(clippy::cast_possible_truncation)]
    data.write_u32_le(path.len() as u32);
    for p in path {
        write_vec3(data, *p);
    }
    if move_spline.is_cyclic() {
        write_vec3(data, Vec3::ZERO);
    } else {
        write_vec3(data, move_spline.final_destination());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::move_spline::MoveSplineInitArgs;

    fn make(path: Vec<Vec3>, velocity: f32, flags: MoveSplineFlag) -> MoveSpline {
        let args = MoveSplineInitArgs {
            path,
            velocity,
            spline_id: 7,
            flags,
            ..MoveSplineInitArgs::default()
        };
        let mut ms = MoveSpline::new();
        ms.initialize(&args);
        ms
    }

    fn straight3() -> Vec<Vec3> {
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
        ]
    }

    #[test]
    fn linear_packet_hand_derived() {
        // Derived by hand from packet_builder.cpp:
        // storage = [(-10,0,0) virtual, (0,0,0), (10,0,0), (20,0,0), (20,0,0) virtual],
        // first=1, last=3, timestamps [0,0,1001,2001].
        // Common part: start=point(1)=(0,0,0), id 7, facing Normal=0,
        // flags=(0 & ~MASK)|RUNMODE=0x100, duration placeholder=2001.
        // LinearPath(firstPoint=0): last_idx = 5-3 = 2 (uncapped),
        // real_path=&point(1); dest=real_path[2]=(20,0,0).
        // count written BEFORE destination: 2-0+1=3.
        // midpoints i=0: offset=(20,0,0) → pack x=lround(80)=80 → bits0-10: 80.
        //           i=1: offset=(10,0,0) → x=40 → 40.
        // Duration back-patch: lastNode=-1 → Duration(0,last=3)-0 = 2001.
        let ms = make(straight3(), 10.0, MoveSplineFlag::default());
        let mut w = PacketWriter::new();
        let r = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        assert_eq!(r, -1);
        let bytes = w.into_vec();

        let mut e: Vec<u8> = Vec::new();
        e.extend_from_slice(&0.0f32.to_le_bytes()); // start xyz
        e.extend_from_slice(&0.0f32.to_le_bytes());
        e.extend_from_slice(&0.0f32.to_le_bytes());
        e.extend_from_slice(&7u32.to_le_bytes()); // spline id
        e.push(0); // MonsterMoveNormal
        e.extend_from_slice(&0x100u32.to_le_bytes()); // Runmode
        e.extend_from_slice(&2001u32.to_le_bytes()); // duration
        e.extend_from_slice(&3u32.to_le_bytes()); // count BEFORE destination
        e.extend_from_slice(&20.0f32.to_le_bytes()); // destination xyz
        e.extend_from_slice(&0.0f32.to_le_bytes());
        e.extend_from_slice(&0.0f32.to_le_bytes());
        e.extend_from_slice(&80u32.to_le_bytes()); // packXYZ(20,0,0)
        e.extend_from_slice(&40u32.to_le_bytes()); // packXYZ(10,0,0)
        assert_eq!(bytes, e);
    }

    #[test]
    fn linear_capped_then_continued() {
        // 4 controls, max 1 point/packet.
        // Call 1 (firstPoint=0): last_idx=6-3=3; 3-0>1 → last_idx=1, written=1.
        //   dest=real_path[1]=point(2)=(10,0,0); count=2; last_idx>1 false →
        //   NO midpoints. Duration patch: Duration(0,1)=lengths[1]-lengths[0]=0.
        // Call 2 (firstPoint=2): last_idx=3; 3-2>1 false. dest=real_path[3]=(30,0,0);
        //   count=2; loop i=2..3: offset=(30,0,0)-real_path[2]=(10,0,0) → 40.
        //   Duration patch: Duration(0,last=4)=3001.
        let path = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::new(30.0, 0.0, 0.0),
        ];
        let ms = make(path, 10.0, MoveSplineFlag::default());

        let mut w1 = PacketWriter::new();
        let r1 = write_monster_move(&mut w1, &ms, 0, 1);
        assert_eq!(r1, 1);
        let b1 = w1.into_vec();
        // duration field at offset 12+4+1+4 = 21
        assert_eq!(u32::from_le_bytes(b1[21..25].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(b1[25..29].try_into().unwrap()), 2); // count
        assert_eq!(
            f32::from_le_bytes(b1[29..33].try_into().unwrap()).to_bits(),
            10.0f32.to_bits(),
            "dest x"
        );
        assert_eq!(b1.len(), 41, "no midpoints when capped to last_idx=1");

        let mut w2 = PacketWriter::new();
        let r2 = write_monster_move(&mut w2, &ms, r1 + 1, 1);
        assert_eq!(r2, -1);
        let b2 = w2.into_vec();
        assert_eq!(u32::from_le_bytes(b2[21..25].try_into().unwrap()), 3001);
        assert_eq!(u32::from_le_bytes(b2[25..29].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(b2[41..45].try_into().unwrap()), 40);
        assert_eq!(b2.len(), 45);
    }

    #[test]
    fn near_zero_offset_gets_nudged_not_skipped() {
        // Midpoint (real_path[1]) is 0.1 below destination in z-offset sign:
        // controls (0,0,0),(20,0,0.1),(20,0,0) → i=1 offset = (0,0,-0.1):
        // all |.|<0.25 and z<0 → z += 0.51 → 0.41 → lround(1.64)=2 → 2<<22.
        // i=0 offset = (20,0,0) → 80.
        let path = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.1),
            Vec3::new(20.0, 0.0, 0.0),
        ];
        let ms = make(path, 10.0, MoveSplineFlag::default());
        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        let b = w.into_vec();
        assert_eq!(u32::from_le_bytes(b[41..45].try_into().unwrap()), 80);
        assert_eq!(
            u32::from_le_bytes(b[45..49].try_into().unwrap()),
            2 << 22,
            "[-ZERO] nudge: z -0.1 + 0.51 = 0.41 → lround(1.64)=2 in bits 22+"
        );
    }

    #[test]
    fn catmull_packet_keeps_full_duration_and_raw_points() {
        // FLYING → CatmullRom: common part duration is NOT back-patched;
        // points are controls[1..] as raw f32 triples, count = storage-3 = 2.
        let ms = make(straight3(), 10.0, MoveSplineFlag(MoveSplineFlag::FLYING));
        let mut w = PacketWriter::new();
        let r = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        assert_eq!(r, -1);
        let b = w.into_vec();
        assert_eq!(u32::from_le_bytes(b[17..21].try_into().unwrap()), 0x300); // FLYING|RUNMODE
        assert_eq!(
            i32::from_le_bytes(b[21..25].try_into().unwrap()),
            ms.duration(),
            "full duration, no patch, no time_passed subtraction"
        );
        assert_eq!(u32::from_le_bytes(b[25..29].try_into().unwrap()), 2);
        assert_eq!(
            f32::from_le_bytes(b[29..33].try_into().unwrap()).to_bits(),
            10.0f32.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(b[41..45].try_into().unwrap()).to_bits(),
            20.0f32.to_bits()
        );
        assert_eq!(b.len(), 25 + 4 + 2 * 12);
    }

    #[test]
    fn cyclic_catmull_writes_fake_first_point() {
        // Cyclic storage (3 controls): N+3=6 points, count=6-3=3, writes 4:
        // point(1) fake, then points[1..4] = controls[0..2].
        let ms = make(
            straight3(),
            10.0,
            MoveSplineFlag(MoveSplineFlag::FLYING | MoveSplineFlag::CYCLIC),
        );
        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        let b = w.into_vec();
        // wire flags: fake Enter_Cycle added: FLYING|CYCLIC|ENTER_CYCLE|RUNMODE
        assert_eq!(
            u32::from_le_bytes(b[17..21].try_into().unwrap()),
            0x200 | 0x10_0000 | 0x20_0000 | 0x100
        );
        assert_eq!(u32::from_le_bytes(b[25..29].try_into().unwrap()), 4);
        // point sequence x: 0 (fake), 0, 10, 20
        let xs: Vec<f32> = (0..4)
            .map(|i| f32::from_le_bytes(b[29 + i * 12..33 + i * 12].try_into().unwrap()))
            .collect();
        assert_eq!(xs, vec![0.0, 0.0, 10.0, 20.0]);
    }

    #[test]
    fn cyclic_linear_packet_gets_enter_cycle_bit() {
        // CYCLIC without FLYING still goes through the linear writer,
        // and the common part sets fake Enter_Cycle (0x200000).
        let ms = make(straight3(), 10.0, MoveSplineFlag(MoveSplineFlag::CYCLIC));
        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        let b = w.into_vec();
        assert_eq!(
            u32::from_le_bytes(b[17..21].try_into().unwrap()),
            0x10_0000 | 0x20_0000 | 0x100
        );
    }

    #[test]
    fn facing_angle_and_target_bytes() {
        let mut args = MoveSplineInitArgs {
            path: straight3(),
            velocity: 10.0,
            spline_id: 7,
            ..MoveSplineInitArgs::default()
        };
        args.flags.enable_facing_angle();
        args.facing = FacingInfo::Angle(1.5);
        let mut ms = MoveSpline::new();
        ms.initialize(&args);
        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        let b = w.into_vec();
        assert_eq!(b[16], 4); // MonsterMoveFacingAngle
        assert_eq!(
            f32::from_le_bytes(b[17..21].try_into().unwrap()).to_bits(),
            1.5f32.to_bits()
        );
        // facing bits are masked OUT of wire flags
        assert_eq!(u32::from_le_bytes(b[21..25].try_into().unwrap()), 0x100);

        let mut args = MoveSplineInitArgs {
            path: straight3(),
            velocity: 10.0,
            spline_id: 7,
            ..MoveSplineInitArgs::default()
        };
        args.flags.enable_facing_target();
        args.facing = FacingInfo::Target(0xF130_0000_0000_0042);
        let mut ms = MoveSpline::new();
        ms.initialize(&args);
        let mut w = PacketWriter::new();
        let _ = write_monster_move(&mut w, &ms, 0, DEFAULT_MAX_POINTS_PER_PACKET);
        let b = w.into_vec();
        assert_eq!(b[16], 3); // MonsterMoveFacingTarget
        assert_eq!(
            u64::from_le_bytes(b[17..25].try_into().unwrap()),
            0xF130_0000_0000_0042
        );
    }

    #[test]
    fn write_create_shape() {
        // raw flags, (no facing), timePassed=0, Duration, id(1.12.1),
        // nodes = full storage size (5 incl. virtuals), points, final dest.
        let ms = make(straight3(), 10.0, MoveSplineFlag::default());
        let mut w = PacketWriter::new();
        write_create(&mut w, &ms);
        let b = w.into_vec();
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 2001);
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 5);
        // last 12 bytes = FinalDestination (20,0,0)
        assert_eq!(
            f32::from_le_bytes(b[b.len() - 12..b.len() - 8].try_into().unwrap()).to_bits(),
            20.0f32.to_bits()
        );
        assert_eq!(b.len(), 20 + 5 * 12 + 12);
    }

    #[test]
    fn write_create_uninitialized_is_empty() {
        let ms = MoveSpline::new();
        let mut w = PacketWriter::new();
        write_create(&mut w, &ms);
        assert!(w.into_vec().is_empty());
    }
}
