//! Golden tests: every expected value below is hand-computed from the
//! C++ formulas in `vmangos/src/game/ObjectGuid.h` (constructors :129-130,
//! `GetEntry` :153, `GetCounter` :156, `HasEntry` :216-233,
//! `GetTypeId` :176-201), `ObjectGuid.cpp` (`ClampPlayerGuid` :202-208,
//! generator :78-106, `GetString` :56-73), and
//! `vmangos/src/shared/ByteBuffer.h` (`appendPackGUID` :440-458).

use mangos_guid::{
    GuidGenerator, HighGuid, ObjectGuid, PackedGuidError, TypeId, clamp_player_guid,
};

#[test]
fn creature_guid_layout_matches_cpp_constructor() {
    // (0xF130 << 48) | (123 << 24) | 456
    let guid = ObjectGuid::with_entry(HighGuid::UNIT, 123, 456);
    assert_eq!(guid.raw(), 0xF130_0000_7B00_01C8);
    assert_eq!(guid.high(), HighGuid::UNIT);
    assert_eq!(guid.entry(), 123);
    assert_eq!(guid.counter(), 456);
    assert!(guid.is_creature() && guid.is_unit() && !guid.is_player());
    assert_eq!(guid.type_id(), TypeId::Unit);
    assert_eq!(guid.to_string(), "Creature (Entry: 123 Guid: 456)");
}

#[test]
fn player_guid_is_bare_counter() {
    let guid = ObjectGuid::new(HighGuid::PLAYER, 42);
    assert_eq!(guid.raw(), 42);
    assert_eq!(guid.entry(), 0);
    assert_eq!(guid.counter(), 42);
    assert!(guid.is_player() && guid.is_unit());
    assert_eq!(guid.type_id(), TypeId::Player);
}

#[test]
fn zero_counter_collapses_to_empty() {
    assert_eq!(ObjectGuid::new(HighGuid::UNIT, 0), ObjectGuid::EMPTY);
    assert_eq!(
        ObjectGuid::with_entry(HighGuid::UNIT, 123, 0),
        ObjectGuid::EMPTY
    );
    assert!(ObjectGuid::EMPTY.is_empty());
    assert!(!ObjectGuid::EMPTY.is_player());
    assert_eq!(ObjectGuid::EMPTY.to_string(), "None (Guid: 0)");
}

#[test]
fn non_entry_counter_uses_full_32_bits() {
    let guid = ObjectGuid::new(HighGuid::ITEM, u32::MAX);
    assert_eq!(guid.raw(), 0x4000_0000_FFFF_FFFF);
    assert_eq!(guid.counter(), u32::MAX);
    assert_eq!(HighGuid::ITEM.max_counter(), u32::MAX);
    assert_eq!(HighGuid::UNIT.max_counter(), 0x00FF_FFFF);
}

#[test]
fn unknown_high_defaults_to_entry_bearing() {
    let high = HighGuid(0x9999);
    assert!(high.has_entry());
    assert_eq!(high.type_name(), "<unknown>");
    assert_eq!(TypeId::from_high(high), TypeId::Object);
}

#[test]
fn transport_maps_to_object_type_id_like_cpp() {
    // HIGHGUID_TRANSPORT has no arm in C++ GetTypeId and falls to Object.
    assert_eq!(TypeId::from_high(HighGuid::TRANSPORT), TypeId::Object);
    assert_eq!(
        TypeId::from_high(HighGuid::MO_TRANSPORT),
        TypeId::GameObject
    );
}

#[test]
fn packed_guid_matches_append_pack_guid_bytes() {
    // LE bytes of 0xF130_0000_7B00_01C8: C8 01 00 7B 00 00 30 F1
    // Non-zero indices 0,1,3,6,7 -> mark 0b1100_1011 = 0xCB
    let guid = ObjectGuid::from_raw(0xF130_0000_7B00_01C8);
    let mut out = Vec::new();
    guid.write_packed(&mut out);
    assert_eq!(out, [0xCB, 0xC8, 0x01, 0x7B, 0x30, 0xF1]);

    let (raw, consumed) = ObjectGuid::read_packed_raw(&out).unwrap();
    assert_eq!(raw, guid.raw());
    assert_eq!(consumed, out.len());

    let mut empty = Vec::new();
    ObjectGuid::EMPTY.write_packed(&mut empty);
    assert_eq!(empty, [0]);
    assert_eq!(ObjectGuid::read_packed_raw(&empty).unwrap(), (0, 1));
}

#[test]
fn packed_guid_truncation_is_an_error_not_a_wrap() {
    // Mark promises bytes 0 and 1, only one follows.
    assert_eq!(
        ObjectGuid::read_packed_raw(&[0x03, 0x2A]),
        Err(PackedGuidError::Truncated)
    );
    assert_eq!(
        ObjectGuid::read_packed_raw(&[]),
        Err(PackedGuidError::Truncated)
    );
}

#[test]
fn player_clamp_strips_aliasing_high_bits() {
    // low 42 plus one multiple of 2^32, high still PLAYER (0).
    let aliased = 42u64 + (1 << 32);
    assert_eq!(clamp_player_guid(aliased), 42);
    assert_eq!(ObjectGuid::from_wire_raw(aliased).raw(), 42);
    // Non-player values pass through untouched.
    let creature = 0xF130_0000_7B00_01C8u64;
    assert_eq!(clamp_player_guid(creature), creature);
    // The packed reader path clamps too.
    let mut packed = Vec::new();
    ObjectGuid::from_raw(aliased).write_packed(&mut packed);
    let (guid, _) = ObjectGuid::read_packed(&packed).unwrap();
    assert_eq!(guid.raw(), 42);
}

#[test]
fn generator_recycles_freed_counters_fifo() {
    let mut generator = GuidGenerator::new(HighGuid::UNIT);
    assert_eq!(generator.generate(), Some(1));
    assert_eq!(generator.generate(), Some(2));
    generator.free(7);
    generator.free(9);
    assert_eq!(generator.generate(), Some(7));
    assert_eq!(generator.generate(), Some(9));
    assert_eq!(generator.generate(), Some(3));
}

#[test]
fn generator_resumes_after_max_used_and_ranges_by_100() {
    let mut generator = GuidGenerator::new(HighGuid::ITEM);
    assert!(generator.set_max_used(41));
    assert_eq!(generator.next_after_max_used(), 42);
    assert_eq!(generator.generate(), Some(42));

    let mut ranged = GuidGenerator::new(HighGuid::ITEM);
    assert_eq!(ranged.generate_range(), Some((1, 101)));
    assert_eq!(ranged.next_after_max_used(), 101);

    let mut exhausted = GuidGenerator::new(HighGuid::ITEM);
    assert!(!exhausted.set_max_used(u32::MAX));
}

#[test]
fn generator_overflow_yields_none_but_freed_still_serve() {
    // Entry-bearing max is 0xFF_FFFF; C++ refuses at next >= max - 1.
    let mut generator = GuidGenerator::with_start(HighGuid::UNIT, 0x00FF_FFFE);
    assert_eq!(generator.generate(), None);
    generator.free(9);
    assert_eq!(generator.generate(), Some(9));
    assert_eq!(generator.generate(), None);
    assert_eq!(
        GuidGenerator::with_start(HighGuid::UNIT, 0x00FF_FFFF - 100).generate_range(),
        None
    );
}
