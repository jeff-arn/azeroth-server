//! World wire tests. Expected values are hand-derived from
//! `AuthCrypt.cpp`, `WorldSocket.cpp`, and an independent SHA1
//! calculation rather than from this crate's output.

use mangos_protocol::world::{
    AuthCrypt, ClientHeader, MAX_SERVER_PAYLOAD_SIZE, MalformedHeader, NUM_MSG_TYPES,
    PacketProcessing, SessionStatus, WorldPacket, auth_response, auth_session_digest_matches,
    build_auth_challenge, build_auth_response, build_pong, encode_server_header,
    expected_auth_session_digest, opcode, parse_auth_session, parse_ping, session_key_from_hex,
};

fn crypt_with_key(key: &[u8]) -> AuthCrypt {
    let mut crypt = AuthCrypt::new();
    crypt.set_key(key);
    crypt.init();
    crypt
}

#[test]
fn auth_crypt_hand_derived_send_vector() {
    let mut crypt = crypt_with_key(&[1, 2, 3]);
    let mut data = [0x10, 0x20, 0x30, 0x40, 0x55];
    crypt.encrypt_send(&mut data);
    // (plain XOR key) + previous cipher, key indices 0,1,2,0.
    assert_eq!(data, [0x11, 0x33, 0x66, 0xA7, 0x55]);
}

#[test]
fn auth_crypt_hand_derived_recv_vector() {
    let mut crypt = crypt_with_key(&[1, 2, 3]);
    let mut data = [0x01, 0x0B, 0xF9, 0xF9, 0xFB, 0xFE, 0x77];
    crypt.decrypt_recv(&mut data);
    assert_eq!(data, [0x00, 0x08, 0xED, 0x01, 0x00, 0x00, 0x77]);
}

#[test]
fn auth_crypt_uninitialized_and_short_inputs_are_noops() {
    let mut crypt = AuthCrypt::new();
    crypt.set_key(&[9, 8, 7]);
    let mut recv = [1, 2, 3, 4, 5, 6];
    crypt.decrypt_recv(&mut recv);
    assert_eq!(recv, [1, 2, 3, 4, 5, 6]);

    crypt.init();
    let mut short_recv = [1, 2, 3, 4, 5];
    let mut short_send = [1, 2, 3];
    crypt.decrypt_recv(&mut short_recv);
    crypt.encrypt_send(&mut short_send);
    assert_eq!(short_recv, [1, 2, 3, 4, 5]);
    assert_eq!(short_send, [1, 2, 3]);
}

#[test]
fn client_header_plaintext_decode_and_validation() {
    let mut crypt = AuthCrypt::new();
    let mut wire = [0x00, 0x08, 0xED, 0x01, 0x00, 0x00];
    let header = ClientHeader::decode(&mut crypt, &mut wire).unwrap();
    assert_eq!(header.size, 8);
    assert_eq!(header.opcode, u32::from(opcode::CMSG_AUTH_SESSION));
    assert_eq!(header.payload_len(), 4);

    let mut too_small = [0x00, 0x03, 0xED, 0x01, 0x00, 0x00];
    assert_eq!(
        ClientHeader::decode(&mut crypt, &mut too_small),
        Err(MalformedHeader::SizeTooSmall {
            size: 3,
            opcode: 493,
        })
    );

    let mut too_large = [0x28, 0x01, 0xED, 0x01, 0x00, 0x00];
    assert_eq!(
        ClientHeader::decode(&mut crypt, &mut too_large),
        Err(MalformedHeader::SizeTooLarge {
            size: 0x2801,
            opcode: 493,
        })
    );

    let mut bad_opcode = [0x00, 0x04, 0x3C, 0x03, 0x00, 0x00]; // 828
    assert_eq!(
        ClientHeader::decode(&mut crypt, &mut bad_opcode),
        Err(MalformedHeader::OpcodeOutOfRange {
            size: 4,
            opcode: NUM_MSG_TYPES as u32,
        })
    );
}

#[test]
fn client_header_encrypted_decode() {
    let mut crypt = crypt_with_key(&[1, 2, 3]);
    let mut wire = [0x01, 0x0B, 0xF9, 0xF9, 0xFB, 0xFE];
    let header = ClientHeader::decode(&mut crypt, &mut wire).unwrap();
    assert_eq!(header.size, 8);
    assert_eq!(header.opcode, 493);
}

#[test]
fn server_header_hand_derived_encryption() {
    let mut plain = AuthCrypt::new();
    let header = encode_server_header(&mut plain, opcode::SMSG_AUTH_CHALLENGE, 4).unwrap();
    assert_eq!(header, [0x00, 0x06, 0xEC, 0x01]);

    let mut crypt = crypt_with_key(&[1, 2, 3]);
    let header = encode_server_header(&mut crypt, opcode::SMSG_AUTH_CHALLENGE, 4).unwrap();
    assert_eq!(header, [0x01, 0x05, 0xF4, 0xF4]);
    assert!(encode_server_header(&mut crypt, 1, MAX_SERVER_PAYLOAD_SIZE + 1).is_err());
}

#[test]
fn world_packet_challenge_and_response_frames() {
    let mut crypt = AuthCrypt::new();
    let packet = build_auth_challenge(0x1122_3344);
    assert_eq!(packet.opcode(), opcode::SMSG_AUTH_CHALLENGE);
    assert_eq!(packet.body(), &[0x44, 0x33, 0x22, 0x11]);
    assert_eq!(
        packet.encode(&mut crypt).unwrap(),
        vec![0x00, 0x06, 0xEC, 0x01, 0x44, 0x33, 0x22, 0x11]
    );

    let response = build_auth_response(auth_response::BANNED);
    assert_eq!(response.opcode(), opcode::SMSG_AUTH_RESPONSE);
    assert_eq!(response.body(), &[28]);
}

#[test]
fn ping_parse_and_pong_build() {
    let ping = parse_ping(&[0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]).unwrap();
    assert_eq!(ping.ping, 0x1122_3344);
    assert_eq!(ping.latency, 0x5566_7788);
    assert!(parse_ping(&[0; 7]).is_err());

    let reply = build_pong(ping.ping);
    assert_eq!(reply.opcode(), opcode::SMSG_PONG);
    assert_eq!(reply.body(), &[0x44, 0x33, 0x22, 0x11]);
}

#[test]
fn auth_session_parse_preserves_addon_tail() {
    let digest = [0xA5; 20];
    let mut body = Vec::new();
    body.extend_from_slice(&5875u32.to_le_bytes());
    body.extend_from_slice(&7u32.to_le_bytes());
    body.extend_from_slice(b"TESTUSER\0");
    body.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
    body.extend_from_slice(&digest);
    body.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

    let packet = parse_auth_session(&body).unwrap();
    assert_eq!(packet.client_build, 5875);
    assert_eq!(packet.server_id, 7);
    assert_eq!(packet.account, "TESTUSER");
    assert_eq!(packet.client_seed, 0x1122_3344);
    assert_eq!(packet.digest, digest);
    assert_eq!(packet.addon_data, [0xDE, 0xAD, 0xBE, 0xEF]);
    assert!(parse_auth_session(&body[..body.len() - 8]).is_err());
}

#[test]
fn auth_session_digest_uses_minimal_le_session_key() {
    // Independent hashlib vector for K=0x0102:
    // SHA1("TESTUSER" || 00000000 || 44332211 || 88776655 || 0201).
    let expected: [u8; 20] = [
        111, 41, 242, 235, 194, 180, 210, 59, 225, 220, 130, 166, 201, 126, 111, 160, 119, 134, 74,
        80,
    ];
    let session_key = session_key_from_hex("0102").unwrap();
    assert_eq!(
        expected_auth_session_digest("TESTUSER", 0x1122_3344, 0x5566_7788, &session_key),
        expected
    );

    let mut body = Vec::new();
    body.extend_from_slice(&5875u32.to_le_bytes());
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(b"TESTUSER\0");
    body.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
    body.extend_from_slice(&expected);
    let packet = parse_auth_session(&body).unwrap();
    assert!(auth_session_digest_matches(
        &packet,
        0x5566_7788,
        &session_key
    ));
    assert!(!auth_session_digest_matches(
        &packet,
        0x5566_7789,
        &session_key
    ));
}

#[test]
fn status_and_processing_wire_values() {
    assert_eq!(SessionStatus::Authed as u8, 0);
    assert_eq!(SessionStatus::Never as u8, 4);
    assert_eq!(PacketProcessing::World as u8, 0);
    assert_eq!(PacketProcessing::MaxType as u8, 5);
    assert_eq!(PacketProcessing::DB_QUERY, PacketProcessing::Async);
    assert_eq!(PacketProcessing::SELF_ITEMS, PacketProcessing::Map);
}

#[test]
fn world_packet_initialize_and_reader() {
    let mut packet = WorldPacket::received(1, [9, 8, 7], 1234);
    packet.initialize(2, 16);
    assert_eq!(packet.opcode(), 2);
    assert!(packet.body().is_empty());
    assert_eq!(packet.received_ms(), 1234);
    packet.body_mut().extend_from_slice(&42u32.to_le_bytes());
    assert_eq!(packet.reader().read_u32_le().unwrap(), 42);
}
