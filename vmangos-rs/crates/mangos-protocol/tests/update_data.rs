//! `UpdateData`/`MovementData`/`UpdateMask` behavior, derived from
//! `UpdateData.cpp:39-57,115-194,203-243` and `UpdateMask.h`.

use std::io::Read as _;

use mangos_guid::ObjectGuid;
use mangos_protocol::update_data::{
    CompressionConfig, MAX_UNCOMPRESSED_PACKET_SIZE, MovementData, ObjectUpdateType,
    SMSG_COMPRESSED_MOVES, SMSG_COMPRESSED_UPDATE_OBJECT, SMSG_UPDATE_OBJECT, UpdateData,
};
use mangos_protocol::update_mask::UpdateMask;
use mangos_protocol::world::WorldPacket;

fn inflate(body: &[u8]) -> Vec<u8> {
    let raw_len = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
    let mut decoder = flate2::read::ZlibDecoder::new(&body[4..]);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).expect("valid zlib stream");
    assert_eq!(out.len(), raw_len, "prefix must be the raw size");
    out
}

#[test]
fn small_packet_is_uncompressed_with_header() {
    let mut ud = UpdateData::new();
    ud.add_update_block().write_bytes(&[1, 2, 3]);
    ud.add_update_block().write_bytes(&[4]);
    assert!(ud.has_data());

    let packet = ud
        .build_first_packet(false, CompressionConfig::default())
        .expect("builds");
    assert_eq!(packet.opcode(), SMSG_UPDATE_OBJECT);
    // u32 count=2, u8 transport=0, then block bytes.
    assert_eq!(packet.body(), &[2, 0, 0, 0, 0, 1, 2, 3, 4]);
}

#[test]
fn out_of_range_section_precedes_blocks_and_bumps_count() {
    let mut ud = UpdateData::new();
    let hi = ObjectGuid::from_raw(0xF130_0000_0000_0009);
    let lo = ObjectGuid::from_raw(0xF130_0000_0000_0003);
    ud.add_out_of_range_guid(hi);
    ud.add_out_of_range_guid(lo);
    ud.add_update_block().write_u8(9);

    let packet = ud
        .build_first_packet(true, CompressionConfig::default())
        .expect("builds");
    let body = packet.body();
    assert_eq!(&body[0..4], &2u32.to_le_bytes(), "1 block + out-of-range");
    assert_eq!(body[4], 1, "hasTransport");
    assert_eq!(body[5], ObjectUpdateType::OutOfRangeObjects as u8);
    assert_eq!(&body[6..10], &2u32.to_le_bytes());
    // Set order: lo guid first (packed: mask + bytes).
    let mut want = Vec::new();
    lo.write_packed(&mut want);
    assert_eq!(&body[10..10 + want.len()], want.as_slice());
    assert_eq!(*body.last().unwrap(), 9, "block bytes come last");
}

#[test]
fn large_payload_compresses_above_threshold() {
    let mut ud = UpdateData::new();
    ud.add_update_block().write_bytes(&[7u8; 200]);

    let packet = ud
        .build_first_packet(false, CompressionConfig::default())
        .expect("builds");
    assert_eq!(packet.opcode(), SMSG_COMPRESSED_UPDATE_OBJECT);
    let raw = inflate(packet.body());
    assert_eq!(raw.len(), 5 + 200);
    assert_eq!(&raw[0..4], &1u32.to_le_bytes());
    assert!(raw[5..].iter().all(|&b| b == 7));

    // Exactly at the threshold (128) stays raw: strictly-greater rule.
    let mut ud = UpdateData::new();
    ud.add_update_block().write_bytes(&[7u8; 123]);
    let packet = ud
        .build_first_packet(false, CompressionConfig::default())
        .expect("builds");
    assert_eq!(packet.body().len(), 128);
    assert_eq!(packet.opcode(), SMSG_UPDATE_OBJECT);
}

#[test]
fn split_happens_before_block_when_already_over_cap() {
    let mut ud = UpdateData::new();
    // One oversized block stays whole in the first sub-packet
    // (the cap is checked before appending, UpdateData.cpp:49).
    ud.add_update_block()
        .write_bytes(&vec![1u8; MAX_UNCOMPRESSED_PACKET_SIZE + 10]);
    assert_eq!(ud.packets().len(), 1);
    // The next block starts a fresh sub-packet.
    ud.add_update_block().write_u8(2);
    assert_eq!(ud.packets().len(), 2);
    assert_eq!(ud.packets()[0].block_count(), 1);
    assert_eq!(ud.packets()[1].block_count(), 1);
}

#[test]
fn build_packets_sends_out_of_range_only_first() {
    let mut ud = UpdateData::new();
    ud.add_out_of_range_guid(ObjectGuid::from_raw(5));
    ud.add_update_block()
        .write_bytes(&vec![1u8; MAX_UNCOMPRESSED_PACKET_SIZE + 1]);
    ud.add_update_block().write_u8(3);

    let packets = ud.build_packets(
        false,
        CompressionConfig {
            level: 1,
            update_size: usize::MAX, // keep raw for byte inspection
        },
    );
    assert_eq!(packets.len(), 2);
    assert_eq!(
        &packets[0].body()[0..4],
        &2u32.to_le_bytes(),
        "first packet carries the out-of-range section"
    );
    assert_eq!(
        &packets[1].body()[0..4],
        &1u32.to_le_bytes(),
        "second packet must not repeat it"
    );
    assert!(ud.out_of_range_guids().is_empty());
}

#[test]
fn pure_out_of_range_packet() {
    let mut ud = UpdateData::new();
    ud.add_out_of_range_guid(ObjectGuid::from_raw(7));
    let packets = ud.build_packets(false, CompressionConfig::default());
    assert_eq!(packets.len(), 1);
    assert_eq!(&packets[0].body()[0..4], &1u32.to_le_bytes());
    assert!(!ud.has_data() || ud.out_of_range_guids().is_empty());
}

#[test]
fn movement_data_batches_with_u8_lengths() {
    let mut md = MovementData::new();
    let a = WorldPacket::with_body(0x00DD, vec![1u8, 2, 3]);
    let big = WorldPacket::with_body(0x00DD, vec![0u8; 254]);
    assert!(md.can_add_packet(&a));
    assert!(!md.can_add_packet(&big), "254+2 exceeds the u8 length");

    md.add_packet(&a);
    md.add_packet(&a);
    assert!(md.has_data());
    let packet = md
        .build_packet(CompressionConfig::default())
        .expect("builds");
    assert_eq!(packet.opcode(), SMSG_COMPRESSED_MOVES);
    let raw = inflate(packet.body());
    // Per packet: u8 len (3+2), u16 opcode LE, body.
    assert_eq!(raw, [5, 0xDD, 0x00, 1, 2, 3, 5, 0xDD, 0x00, 1, 2, 3]);

    md.clear();
    assert!(!md.has_data());
}

#[test]
fn update_mask_bit_layout() {
    let mut mask = UpdateMask::new();
    mask.set_count(65);
    assert_eq!(mask.block_count(), 3);
    assert_eq!(mask.len_bytes(), 12);
    assert!(!mask.has_data());

    mask.set_bit(0);
    mask.set_bit(31);
    mask.set_bit(32);
    mask.set_bit(64);
    assert_eq!(mask.blocks(), &[0x8000_0001, 0x0000_0001, 0x0000_0001]);
    assert!(mask.has_data());
    assert!(mask.get_bit(64));

    mask.unset_bit(0);
    assert!(!mask.get_bit(0));
    assert!(mask.has_data(), "unset does not recompute has_data (C++)");

    let mut other = UpdateMask::new();
    other.set_count(65);
    other.set_bit(31);
    other.set_bit(63);
    mask &= &other;
    assert_eq!(mask.blocks(), &[0x8000_0000, 0, 0]);
    assert!(mask.has_data());
    mask |= &other;
    assert!(mask.get_bit(63));

    mask.clear();
    assert!(!mask.has_data());
    assert_eq!(mask.blocks(), &[0, 0, 0]);
}
