//! Behavior tests for the values word store, derived from
//! `Object.cpp:942-972,1035-1313,2806-2810` and `Object.h:175-355`.

use mangos_entities::Values;
use mangos_protocol::update_fields::UpdateFieldFlags;
use mangos_protocol::update_mask::UpdateMask;

#[test]
fn changed_only_writes_report_change() {
    let mut v = Values::new(8);
    assert!(!v.set_u32(0, 0), "no-op write must not report change");
    assert!(v.set_u32(0, 5));
    assert!(!v.set_u32(0, 5));
    assert_eq!(v.get_u32(0), 5);

    assert!(v.set_i32(1, -3));
    assert_eq!(v.get_i32(1), -3);
    assert_eq!(v.get_u32(1), (-3i32) as u32);
}

#[test]
fn float_nan_always_changed() {
    let mut v = Values::new(4);
    assert!(v.set_f32(0, 1.5));
    assert!(!v.set_f32(0, 1.5));
    assert!(v.set_f32(0, f32::NAN));
    assert!(v.set_f32(0, f32::NAN), "NaN != NaN in C++, always dirty");
}

#[test]
fn u64_set_poisons_both_mirror_halves() {
    let mut v = Values::new(8);
    assert!(v.set_u64(2, 0xAAAA_BBBB_CCCC_DDDD));
    assert_eq!(v.get_u64(2), 0xAAAA_BBBB_CCCC_DDDD);
    assert_eq!(v.words()[2], 0xCCCC_DDDD);
    assert_eq!(v.words()[3], 0xAAAA_BBBB);
    assert_eq!(v.mirror()[2], 0xCCCC_DDDD + 1);
    assert_eq!(v.mirror()[3], 0xAAAA_BBBB + 1);
    assert!(
        !v.set_u64(2, 0xAAAA_BBBB_CCCC_DDDD),
        "same value: no change"
    );
}

#[test]
fn guid_helpers_use_u64_lane() {
    use mangos_guid::ObjectGuid;
    let mut v = Values::new(6);
    let g = ObjectGuid::from_raw(0xF130_0001_0000_0042);
    assert!(v.set_guid(0, g));
    assert_eq!(v.get_guid(0), g);
}

#[test]
fn byte_and_short_lanes_are_little_endian() {
    let mut v = Values::new(4);
    assert!(v.set_u32(0, 0xAABB_CCDD));
    assert_eq!(v.get_byte(0, 0), 0xDD);
    assert_eq!(v.get_byte(0, 3), 0xAA);
    assert_eq!(v.get_u16(0, 0), 0xCCDD);
    assert_eq!(v.get_u16(0, 1), 0xAABB);
    assert_eq!(v.get_i16(0, 1), i16::from_ne_bytes(0xAABBu16.to_ne_bytes()));

    assert!(v.set_byte(0, 1, 0x11));
    assert_eq!(v.get_u32(0), 0xAABB_11DD);
    assert!(!v.set_byte(0, 1, 0x11));
    assert!(v.set_i16(0, 1, -1));
    assert_eq!(v.get_u32(0), 0xFFFF_11DD);
}

#[test]
fn clear_update_mask_syncs_mirror_and_force_re_dirties() {
    let mut v = Values::new(4);
    v.set_u32(1, 10);
    let flags = [UpdateFieldFlags::PUBLIC; 4];
    assert!(v.update_bits(&flags, UpdateFieldFlags::PUBLIC).get_bit(1));

    v.clear_update_mask();
    assert!(!v.update_bits(&flags, UpdateFieldFlags::PUBLIC).has_data());

    v.force_update_at(1);
    assert_eq!(v.mirror()[1], 11);
    assert!(v.update_bits(&flags, UpdateFieldFlags::PUBLIC).get_bit(1));
}

#[test]
fn create_and_update_bits_respect_visibility() {
    let mut v = Values::new(4);
    v.set_u32(0, 1); // public
    v.set_u32(1, 2); // private
    v.set_u32(2, 0); // zero word: never in create mask
    let flags = [
        UpdateFieldFlags::PUBLIC,
        UpdateFieldFlags::PRIVATE,
        UpdateFieldFlags::PUBLIC,
        UpdateFieldFlags::PUBLIC,
    ];

    let public = v.create_bits(&flags, UpdateFieldFlags::PUBLIC);
    assert!(public.get_bit(0));
    assert!(!public.get_bit(1), "private hidden from public viewer");
    assert!(!public.get_bit(2));

    let self_view = v.create_bits(&flags, UpdateFieldFlags::PUBLIC | UpdateFieldFlags::PRIVATE);
    assert!(self_view.get_bit(1));

    v.clear_update_mask();
    v.set_u32(1, 3);
    let upd = v.update_bits(&flags, UpdateFieldFlags::PUBLIC);
    assert!(!upd.has_data(), "private-only change invisible publicly");
}

#[test]
fn mark_fields_with_flag_honours_empty_rule() {
    let mut v = Values::new(4);
    v.set_u32(0, 7);
    let flags = [UpdateFieldFlags::DYNAMIC; 4];

    let mut mask = UpdateMask::new();
    mask.set_count(4);
    v.mark_fields_with_flag(&mut mask, &flags, UpdateFieldFlags::DYNAMIC, false);
    assert!(mask.get_bit(0));
    assert!(
        !mask.get_bit(1),
        "zero words skipped unless including_empty"
    );

    let mut mask = UpdateMask::new();
    mask.set_count(4);
    v.mark_fields_with_flag(&mut mask, &flags, UpdateFieldFlags::DYNAMIC, true);
    assert!(mask.get_bit(1));
}

#[test]
fn flag_ops() {
    let mut v = Values::new(6);
    assert!(v.set_flag(0, 0b0100));
    assert!(!v.set_flag(0, 0b0100));
    assert!(v.has_flag(0, 0b0100));
    assert!(v.toggle_flag(0, 0b0100));
    assert!(!v.has_flag(0, 0b0100));
    assert!(v.apply_mod_flag(0, 0b0001, true));
    assert!(v.remove_flag(0, 0b0001));

    // Byte flags: partial intersect is "already set" for set (C++).
    assert!(v.set_byte_flag(1, 2, 0b0011));
    assert_eq!(v.get_byte(1, 2), 0b0011);
    assert!(!v.set_byte_flag(1, 2, 0b0001), "any-bit set means no-op");
    assert!(v.has_byte_flag(1, 2, 0b0010));
    assert!(v.remove_byte_flag(1, 2, 0b0010));
    assert_eq!(v.get_byte(1, 2), 0b0001);
    assert!(v.toggle_byte_flag(1, 2, 0b0001));
    assert_eq!(v.get_byte(1, 2), 0);

    assert!(v.set_short_flag(2, true, 0x8001));
    assert_eq!(v.get_u32(2), 0x8001_0000);
    // C++ HasShortFlag takes a uint8 flag: it can only ever see the
    // lane's low byte (0x8000 is invisible to it, 0x0001 is not).
    assert!(v.has_short_flag(2, true, 0x01));
    assert!(
        !v.has_short_flag(2, true, 0x80),
        "0x8000 & 0x80 == 0: high byte invisible to the u8-flag test"
    );
    assert!(v.remove_short_flag(2, true, 0x8001));

    assert!(v.set_flag64(3, 1 << 40));
    assert!(v.has_flag64(3, 1 << 40));
    assert!(v.toggle_flag64(3, 1 << 40));
    assert!(!v.has_flag64(3, 1 << 40));
}

#[test]
fn apply_mods_clamp_and_wrap_like_cpp() {
    let mut v = Values::new(6);
    v.set_u32(0, 10);
    assert!(v.apply_mod_u32(0, 5, true));
    assert_eq!(v.get_u32(0), 15);
    assert!(v.apply_mod_u32(0, 30, false));
    assert_eq!(v.get_u32(0), 0, "clamped at zero, never wraps negative");

    v.set_i32(1, -5);
    assert!(v.apply_mod_i32(1, 3, false));
    assert_eq!(v.get_i32(1), -8);

    v.set_f32(2, 2.0);
    assert!(v.apply_mod_positive_f32(2, 5.0, false));
    assert_eq!(v.get_f32(2).to_bits(), 0.0f32.to_bits());

    v.set_f32(3, 100.0);
    assert!(v.apply_percent_mod_f32(3, 50.0, true));
    assert_eq!(v.get_f32(3).to_bits(), 150.0f32.to_bits());
    assert!(v.apply_percent_mod_f32(3, 50.0, false));
    assert_eq!(v.get_f32(3).to_bits(), 100.0f32.to_bits());
    // -100% would divide by zero unapplying; C++ rewrites to -99.9.
    assert!(v.apply_percent_mod_f32(3, -100.0, true));
    assert!(v.get_f32(3) > 0.0 && v.get_f32(3) < 1.0);

    assert!(!v.set_stat_f32(4, -2.0), "clamps to existing 0.0");
    assert!(!v.set_stat_i32(5, -9));
}

#[test]
fn load_values_atol_semantics() {
    let mut v = Values::new(4);
    assert!(v.load_values("1 -2 3000000000 4x"));
    assert_eq!(
        v.words(),
        &[
            1,
            u32::from_ne_bytes((-2i32).to_ne_bytes()),
            3_000_000_000,
            4
        ]
    );
    // Double spaces are skipped by StrSplit.
    assert!(v.load_values("9  8   7 6"));
    assert_eq!(v.words(), &[9, 8, 7, 6]);
    // Wrong count writes nothing.
    assert!(!v.load_values("1 2 3"));
    assert_eq!(v.words(), &[9, 8, 7, 6]);
    // Mirror untouched by load (stays zero here, so all dirty).
    assert_eq!(v.mirror(), &[0, 0, 0, 0]);
}

#[test]
fn load_into_data_field_poisons_mirror() {
    let mut v = Values::new(8);
    v.load_into_data_field("10 20", 3, 2);
    assert_eq!(v.words()[3], 10);
    assert_eq!(v.mirror()[3], 11);
    assert_eq!(v.words()[4], 20);
    // Mismatch and empty input are silent no-ops.
    v.load_into_data_field("1 2 3", 0, 2);
    v.load_into_data_field("", 0, 0);
    assert_eq!(v.words()[0], 0);
}
