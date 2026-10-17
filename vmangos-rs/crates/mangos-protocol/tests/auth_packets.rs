//! Packet layout tests: expected bytes are hand-assembled from
//! `AuthPackets.h` packed structs and the `AuthSocket.cpp` builders.

use mangos_protocol::auth::{
    build_logon_challenge_success, build_logon_proof_success, parse_logon_challenge_body,
    parse_logon_challenge_header, parse_logon_proof, parse_reconnect_proof,
};
use mangos_protocol::buffer::PacketReader;

#[test]
fn challenge_body_parses_packed_layout() {
    // Hand-assembled sAuthLogonChallengeBody for build 5875, "WoW",
    // version 1.12.1, x86/Win/enUS, username TESTUSER.
    let mut body = Vec::new();
    body.extend_from_slice(b"WoW\0");
    body.extend_from_slice(&[1, 12, 1]);
    body.extend_from_slice(&5875u16.to_le_bytes());
    body.extend_from_slice(b"x86\0");
    body.extend_from_slice(b"Win\0");
    body.extend_from_slice(b"enUS");
    body.extend_from_slice(&0xFFFF_FFF8u32.to_le_bytes()); // timezone bias -8 min-ish
    body.extend_from_slice(&0x0100_007Fu32.to_le_bytes()); // ip field as sent
    body.push(8);
    body.extend_from_slice(b"TESTUSER");
    let parsed = parse_logon_challenge_body(&body).unwrap();
    assert_eq!(parsed.build, 5875);
    assert_eq!(parsed.version, (1, 12, 1));
    assert_eq!(&parsed.gamename, b"WoW\0");
    assert_eq!(parsed.username, "TESTUSER");
}

#[test]
fn challenge_header_parses() {
    let mut reader = PacketReader::new(&[0x00, 0x31, 0x00]);
    let header = parse_logon_challenge_header(&mut reader).unwrap();
    assert_eq!(header.error, 0);
    assert_eq!(header.size, 0x31);
}

#[test]
fn challenge_success_packet_byte_order() {
    let b32 = [0xBBu8; 32];
    let n32 = [0xAAu8; 32];
    let salt = [0x11u8; 32];
    let pkt = build_logon_challenge_success(&b32, &[7], &n32, &salt, 5875, None);
    assert_eq!(pkt[0], 0x00); // CMD_AUTH_LOGON_CHALLENGE
    assert_eq!(pkt[1], 0x00); // fixed 0x00 prefix (AuthSocket.cpp:281)
    assert_eq!(pkt[2], 0x00); // WOW_SUCCESS
    assert_eq!(&pkt[3..35], &b32);
    assert_eq!(pkt[35], 1); // g_len
    assert_eq!(pkt[36], 7); // g
    assert_eq!(pkt[37], 32); // N_len
    assert_eq!(&pkt[38..70], &n32);
    assert_eq!(&pkt[70..102], &salt);
    assert_eq!(&pkt[102..118], &mangos_protocol::auth::VERSION_CHALLENGE);
    assert_eq!(pkt[118], 0); // security flags placeholder for build >= 5428
    assert_eq!(pkt.len(), 119);
}

#[test]
fn logon_proof_parse_with_and_without_pin() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xAAu8; 32]);
    data.extend_from_slice(&[0xBBu8; 20]);
    data.extend_from_slice(&[0xCCu8; 20]);
    data.push(0); // number_of_keys
    data.push(1); // security flags: PIN
    data.extend_from_slice(&[0xDDu8; 16]);
    data.extend_from_slice(&[0xEEu8; 20]);
    let proof = parse_logon_proof(&data, 5875).unwrap();
    assert_eq!(proof.a, [0xAAu8; 32]);
    assert_eq!(proof.m1, [0xBBu8; 20]);
    assert_eq!(proof.pin_salt, Some([0xDDu8; 16]));
    assert_eq!(proof.pin_hash, Some([0xEEu8; 20]));

    // Pre-1.11 build: no securityFlags byte is read.
    let proof_old = parse_logon_proof(&data[..73], 5300).unwrap();
    assert_eq!(proof_old.security_flags, 0);
    assert_eq!(proof_old.pin_salt, None);
}

#[test]
fn proof_success_packet_5875_layout() {
    let m2 = [0x42u8; 20];
    let pkt = build_logon_proof_success(5875, &m2);
    assert_eq!(pkt.len(), 26);
    assert_eq!(pkt[0], 0x01);
    assert_eq!(pkt[1], 0x00);
    assert_eq!(&pkt[2..22], &m2);
    assert_eq!(&pkt[22..26], &[0, 0, 0, 0]); // surveyId
}

#[test]
fn reconnect_proof_parses() {
    let mut data = Vec::new();
    data.extend_from_slice(&[1u8; 16]);
    data.extend_from_slice(&[2u8; 20]);
    data.extend_from_slice(&[3u8; 20]);
    data.push(0);
    let parsed = parse_reconnect_proof(&data).unwrap();
    assert_eq!(parsed.r1, [1u8; 16]);
    assert_eq!(parsed.r2, [2u8; 20]);
    assert_eq!(parsed.r3, [3u8; 20]);
}

#[test]
fn truncated_packets_error_instead_of_panicking() {
    assert!(parse_logon_challenge_body(&[0u8; 4]).is_err());
    assert!(parse_logon_proof(&[0u8; 10], 5875).is_err());
    assert!(parse_reconnect_proof(&[]).is_err());
}
