//! Golden byte vectors for the `MovementInfo` codec, hand-derived
//! from `Object.cpp:61-172` (never from the Rust implementation):
//! expected buffers are assembled field-by-field with `to_le_bytes`
//! in the C++ wire order, and sizes are the C++ sizes
//! (base 28, +24 transport, +4 swim, +16 jump, +4 spline = 76 max).

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::{
    MAP_HALFSIZE, MovementFlags, MovementInfo, Position, is_valid_map_coord,
};
use mangos_protocol::buffer::{PacketReader, PacketWriter};

fn pos() -> Position {
    Position::new(1.5, -2.25, 300.0, 1.25)
}

fn push_pos(buf: &mut Vec<u8>, p: Position) {
    for v in [p.x, p.y, p.z, p.o] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn sample() -> MovementInfo {
    MovementInfo {
        server_time: 0x1122_3344,
        client_time: 0x5566_7788,
        pos: pos(),
        fall_time: 0xAABB_CCDD,
        ..MovementInfo::default()
    }
}

fn write_bytes(info: &MovementInfo) -> Vec<u8> {
    let mut w = PacketWriter::new();
    info.write(&mut w);
    w.into_vec()
}

#[test]
fn base_block_is_28_bytes_and_writes_server_time() {
    let info = sample();
    let got = write_bytes(&info);

    let mut want = Vec::new();
    want.extend_from_slice(&0u32.to_le_bytes()); // flags NONE
    want.extend_from_slice(&0x1122_3344u32.to_le_bytes()); // stime, NOT ctime
    push_pos(&mut want, pos());
    want.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes()); // fallTime

    assert_eq!(got.len(), 28);
    assert_eq!(got, want);
}

#[test]
fn transport_block_is_raw_guid_then_position() {
    let mut info = sample();
    info.flags.add(MovementFlags::ON_TRANSPORT);
    info.transport_guid = ObjectGuid::from_wire_raw(0xF120_0000_00AB_CDEF);
    let tpos = Position::new(10.0, 20.0, 5.5, 0.5);
    info.transport_pos = tpos;

    let got = write_bytes(&info);
    assert_eq!(got.len(), 52);
    // GUID at offset 24 (after flags/time/pos), un-packed u64.
    assert_eq!(
        &got[24..32],
        &info.transport_guid.raw().to_le_bytes(),
        "transport guid must be raw u64 LE"
    );
    let mut want_tpos = Vec::new();
    push_pos(&mut want_tpos, tpos);
    assert_eq!(&got[32..48], want_tpos.as_slice());
    // fallTime follows the transport block.
    assert_eq!(&got[48..52], &0xAABB_CCDDu32.to_le_bytes());
}

#[test]
fn swim_pitch_precedes_fall_time() {
    let mut info = sample();
    info.flags.add(MovementFlags::SWIMMING);
    info.swim_pitch = -0.75;

    let got = write_bytes(&info);
    assert_eq!(got.len(), 32);
    assert_eq!(&got[24..28], &(-0.75f32).to_le_bytes());
    assert_eq!(&got[28..32], &0xAABB_CCDDu32.to_le_bytes());
}

#[test]
fn jump_block_is_z_cos_sin_xy() {
    let mut info = sample();
    info.flags.add(MovementFlags::JUMPING);
    info.jump.z_speed = 1.0;
    info.jump.cos_angle = 2.0;
    info.jump.sin_angle = 3.0;
    info.jump.xy_speed = 4.0;

    let got = write_bytes(&info);
    assert_eq!(got.len(), 44);
    let mut want_tail = Vec::new();
    for v in [1.0f32, 2.0, 3.0, 4.0] {
        want_tail.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(&got[28..44], want_tail.as_slice(), "cos must precede sin");
}

#[test]
fn spline_elevation_is_last() {
    let mut info = sample();
    info.flags.add(MovementFlags::SPLINE_ELEVATION);
    info.spline_elevation = 9.5;

    let got = write_bytes(&info);
    assert_eq!(got.len(), 32);
    assert_eq!(&got[28..32], &9.5f32.to_le_bytes());
}

#[test]
fn all_conditionals_is_76_bytes_in_cpp_order() {
    let mut info = sample();
    info.flags = MovementFlags(
        MovementFlags::ON_TRANSPORT
            | MovementFlags::SWIMMING
            | MovementFlags::JUMPING
            | MovementFlags::SPLINE_ELEVATION,
    );
    info.transport_guid = ObjectGuid::from_wire_raw(0xF130_0001_0000_0042);
    info.transport_pos = Position::new(1.0, 2.0, 3.0, 4.0);
    info.swim_pitch = 5.0;
    info.jump.z_speed = 6.0;
    info.jump.cos_angle = 7.0;
    info.jump.sin_angle = 8.0;
    info.jump.xy_speed = 9.0;
    info.spline_elevation = 10.0;

    let got = write_bytes(&info);
    assert_eq!(got.len(), 76);

    let mut want = Vec::new();
    want.extend_from_slice(&info.flags.raw().to_le_bytes());
    want.extend_from_slice(&info.server_time.to_le_bytes());
    push_pos(&mut want, info.pos);
    want.extend_from_slice(&info.transport_guid.raw().to_le_bytes());
    push_pos(&mut want, info.transport_pos);
    want.extend_from_slice(&info.swim_pitch.to_le_bytes());
    want.extend_from_slice(&info.fall_time.to_le_bytes());
    for v in [6.0f32, 7.0, 8.0, 9.0] {
        want.extend_from_slice(&v.to_le_bytes());
    }
    want.extend_from_slice(&10.0f32.to_le_bytes());
    assert_eq!(got, want);
}

/// Build a client movement packet by hand (ctime in the time slot).
fn client_packet(flags: u32, ctime: u32, p: Position) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&ctime.to_le_bytes());
    push_pos(&mut buf, p);
    buf.extend_from_slice(&0u32.to_le_bytes()); // fallTime
    buf
}

#[test]
fn read_takes_client_time_and_stamps_server_time() {
    let bytes = client_packet(MovementFlags::NONE, 777, pos());
    let mut info = MovementInfo::default();
    info.read(&mut PacketReader::new(&bytes), 4242)
        .expect("full base packet reads");

    assert_eq!(info.client_time, 777);
    assert_eq!(info.server_time, 4242);
    assert_eq!(info.pos, pos());

    // Relay shape: writing the parsed value puts stime back out.
    let out = write_bytes(&info);
    assert_eq!(&out[4..8], &4242u32.to_le_bytes());
}

#[test]
fn jump_latch_is_seeded_across_packets() {
    let jump_flags = MovementFlags::JUMPING;
    let first_pos = Position::new(10.0, 0.0, 50.0, 0.0);
    let mut first = client_packet(jump_flags, 1000, first_pos);
    for v in [7.0f32, 0.6, 0.8, 7.0] {
        first.extend_from_slice(&v.to_le_bytes());
    }

    let mut info = MovementInfo::default();
    info.read(&mut PacketReader::new(&first), 1).expect("reads");
    assert_eq!(
        info.jump.start_client_time, 1000,
        "latch sets on first jump"
    );
    assert_eq!(info.jump.start, first_pos);
    assert_eq!(info.jump.cos_angle.to_bits(), 0.6f32.to_bits());
    assert_eq!(info.jump.sin_angle.to_bits(), 0.8f32.to_bits());

    // Second jumping packet: moved position, later ctime — the
    // latch must survive because read is seeded from previous state.
    let second_pos = Position::new(12.0, 0.0, 55.0, 0.0);
    let mut second = client_packet(jump_flags, 1500, second_pos);
    for v in [6.0f32, 0.6, 0.8, 7.0] {
        second.extend_from_slice(&v.to_le_bytes());
    }
    info.read(&mut PacketReader::new(&second), 2)
        .expect("reads");
    assert_eq!(info.jump.start_client_time, 1000, "latch must not reset");
    assert_eq!(info.jump.start, first_pos);
    assert_eq!(
        info.jump.z_speed.to_bits(),
        6.0f32.to_bits(),
        "jump values still parse"
    );

    // A non-jumping packet resets only start_client_time.
    let ground = client_packet(MovementFlags::NONE, 2000, second_pos);
    info.read(&mut PacketReader::new(&ground), 3)
        .expect("reads");
    assert_eq!(info.jump.start_client_time, 0);
    assert_eq!(info.jump.start, first_pos, "start pos is left untouched");
    assert_eq!(
        info.jump.z_speed.to_bits(),
        6.0f32.to_bits(),
        "jump speeds are left untouched"
    );
}

#[test]
fn correct_data_strips_contradictory_pairs() {
    let pairs = [
        (MovementFlags::TURN_LEFT, MovementFlags::TURN_RIGHT),
        (MovementFlags::STRAFE_LEFT, MovementFlags::STRAFE_RIGHT),
        (MovementFlags::PITCH_UP, MovementFlags::PITCH_DOWN),
        (MovementFlags::FORWARD, MovementFlags::BACKWARD),
    ];
    for (a, b) in pairs {
        let mut info = MovementInfo {
            flags: MovementFlags(a | b | MovementFlags::WALK_MODE),
            ..MovementInfo::default()
        };
        info.correct_data();
        assert_eq!(
            info.flags.raw(),
            MovementFlags::WALK_MODE,
            "pair {a:#x}/{b:#x} must both strip, unrelated flags stay"
        );
    }

    // ROOT+moving strips ROOT only; ROOT alone stays.
    let mut info = MovementInfo {
        flags: MovementFlags(MovementFlags::ROOT | MovementFlags::FORWARD),
        ..MovementInfo::default()
    };
    info.correct_data();
    assert_eq!(info.flags.raw(), MovementFlags::FORWARD);

    let mut info = MovementInfo {
        flags: MovementFlags(MovementFlags::ROOT),
        ..MovementInfo::default()
    };
    info.correct_data();
    assert_eq!(info.flags.raw(), MovementFlags::ROOT);

    // The HOVER aura check is commented out in C++: HOVER survives.
    let mut info = MovementInfo {
        flags: MovementFlags(MovementFlags::HOVER),
        ..MovementInfo::default()
    };
    info.correct_data();
    assert_eq!(info.flags.raw(), MovementFlags::HOVER);
}

#[test]
fn verify_bounds_match_cpp() {
    let mut info = sample();
    assert!(info.verify());

    info.pos.x = MAP_HALFSIZE; // one past MAP_HALFSIZE - 0.5
    assert!(!info.verify());
    info.pos.x = f32::NAN;
    assert!(!info.verify());
    info.pos = pos();

    info.pos.o = 13.0; // > 4*pi ~= 12.566
    assert!(!info.verify());
    info.pos = pos();

    assert!(is_valid_map_coord(0.0, 0.0, 400_000.0, 0.0));
    assert!(!is_valid_map_coord(0.0, 0.0, 400_001.0, 0.0));
    assert!(!is_valid_map_coord(0.0, 0.0, f32::NAN, 0.0));

    // Transport box + summed-position rules.
    info.flags.add(MovementFlags::ON_TRANSPORT);
    info.transport_pos = Position::new(250.0, -250.0, 100.0, 0.0);
    assert!(info.verify(), "box edges are inclusive");
    info.transport_pos.x = 250.5;
    assert!(!info.verify());
    info.transport_pos = Position::new(10.0, 0.0, 0.0, 0.0);
    info.pos.x = MAP_HALFSIZE - 6.0; // valid alone, sum over the edge
    assert!(!info.verify());
}

#[test]
fn set_as_server_side_preserves_ordering() {
    let mut info = sample();
    info.set_as_server_side(100);
    assert_eq!(info.server_time, 0x1122_3345, "bumps past the stale time");
    assert_eq!(info.client_time, 0);
    assert_eq!(info.source_session_guid, 0);

    info.set_as_server_side(0xFFFF_FF00);
    assert_eq!(info.server_time, 0xFFFF_FF00, "newer time is kept as-is");
}

#[test]
fn truncated_packets_error() {
    let bytes = client_packet(MovementFlags::NONE, 1, pos());
    let mut info = MovementInfo::default();
    assert!(info.read(&mut PacketReader::new(&bytes[..27]), 0).is_err());

    // Transport flag demands 24 more bytes than a base packet has.
    let short = client_packet(MovementFlags::ON_TRANSPORT, 1, pos());
    assert!(info.read(&mut PacketReader::new(&short), 0).is_err());
}

#[test]
fn round_trip_through_wire() {
    let mut info = sample();
    info.flags = MovementFlags(MovementFlags::SWIMMING | MovementFlags::SPLINE_ELEVATION);
    info.swim_pitch = 0.25;
    info.spline_elevation = -3.5;

    let bytes = write_bytes(&info);
    let mut back = MovementInfo::default();
    back.read(&mut PacketReader::new(&bytes), 99)
        .expect("reads");

    assert_eq!(back.pos, info.pos);
    assert_eq!(back.swim_pitch.to_bits(), 0.25f32.to_bits());
    assert_eq!(back.spline_elevation.to_bits(), (-3.5f32).to_bits());
    assert_eq!(back.fall_time, info.fall_time);
    // The time slot asymmetry is observable: the reader sees the
    // writer's server time as its client time.
    assert_eq!(back.client_time, info.server_time);
    assert_eq!(back.server_time, 99);
}
