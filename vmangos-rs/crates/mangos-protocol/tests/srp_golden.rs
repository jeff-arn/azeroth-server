//! SRP golden tests. Verifier vectors come from the Phase 0 corpus
//! (`~/azeroth-dev-fixtures/corpora/srp_vectors.json`), derived from
//! the C++ code path and cross-checked against
//! `dashboard/tests/test_auth.py`. The handshake test then proves
//! internal consistency with an independent client-side computation
//! written from the same C++ semantics.

use mangos_protocol::srp::{
    SrpSession, bn_as_byte_array, bn_from_binary, bn_from_hex, bn_to_hex, calculate_verifier,
    generate_salt, hash_session_key, prime_n, sha_pass_hash,
};
use num_bigint::BigUint;
use sha1::{Digest, Sha1};

fn verifier_for(username: &str, password: &str, salt_hex: &str) -> String {
    let salt = bn_from_hex(salt_hex).expect("salt parses");
    bn_to_hex(&calculate_verifier(
        &sha_pass_hash(username, password),
        &salt,
    ))
}

#[test]
fn oracle_verifier_vectors_match() {
    // dashboard/tests/test_auth.py VECTORS (verified against real logins)
    assert_eq!(
        verifier_for(
            "TESTUSER",
            "password123",
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
        ),
        "68EA0AA0AFC9E06942699D0EB823713252D1BD8B30A1806A3FEFD23B77DB7E34"
    );
    assert_eq!(
        verifier_for(
            "AdminGM",
            "shortpw",
            "DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF"
        ),
        "29F1EDE47B46D96CADD6C14437895F6B726F694E04100DD2622ECD9A30B89CCD"
    );
}

#[test]
fn derived_verifier_vectors_match() {
    // Phase 0 corpus derived vectors (C++-faithful salt feed)
    assert_eq!(
        verifier_for("a", "b", &"FF".repeat(32)),
        "4595CD03474EBC3F8722BF076241AC5711309618C50A2B0FC1F4AB0A25FEA475"
    );
    assert_eq!(
        verifier_for(
            "LONGUSERNAME0123456789",
            "correct horse battery staple",
            "1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF"
        ),
        "324587BB4CB7943A20F093DB7C03811A0AF4A3E11DFB53BDA883AB1AF40E4B1A"
    );
}

#[test]
fn leading_zero_salt_uses_cpp_minimal_feed_not_dashboard_padding() {
    // Salt with a leading 0x00 byte: C++ feeds 31 minimal bytes. The
    // dashboard's padded variant would give a different verifier;
    // the corpus value below is the C++ one (Phase 0 finding).
    assert_eq!(
        verifier_for(
            "RUSTTEST",
            "Phase0!golden",
            "00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF"
        ),
        "6131F44621FF85403300A5EA07DF6CF8DED2C92D30759AC68BC6B025EBB4FDDF"
    );
}

#[test]
fn generated_salt_is_always_32_minimal_bytes() {
    for _ in 0..64 {
        let salt = generate_salt();
        assert_eq!(bn_as_byte_array(&salt).len(), 32);
        assert_eq!(bn_to_hex(&salt).len(), 64);
    }
}

/// Client-side SRP from the same published semantics: proves the
/// server accepts exactly the M1 a conforming client computes, and
/// that both sides derive the same K and M2.
#[test]
#[allow(clippy::many_single_char_names)] // SRP notation: a, b, g, x, u, N are the spec's names
fn full_handshake_client_server_agree() {
    let username = "TESTUSER";
    let password = "password123";
    let salt =
        bn_from_hex("ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789").unwrap();
    let verifier = calculate_verifier(&sha_pass_hash(username, password), &salt);
    let n = prime_n();
    let g = BigUint::from(7u32);

    // Server: deterministic b for the test.
    let b = BigUint::parse_bytes(b"1234567890abcdef1234567890abcdef123456", 16).unwrap();
    let mut server = SrpSession::host_ephemeral_with_b(verifier.clone(), salt.clone(), b);
    let b_pub = bn_from_binary(&server.public_b_bytes32());

    // Client: a, A = g^a; x from password via the verifier formula's
    // inner hash (recomputed here, not via calculate_verifier).
    let a = BigUint::parse_bytes(b"fedcba0987654321fedcba0987654321fedcba", 16).unwrap();
    let big_a = g.modpow(&a, &n);
    let a_wire = mangos_protocol::srp::bn_as_byte_array_padded(&big_a, 32);

    assert!(server.calculate_session_key(&a_wire));
    server.calculate_proof(username);

    // Client x: SHA1(s_LE || mDigest) per CalculateVerifier internals.
    let identity = bn_from_hex(&sha_pass_hash(username, password)).unwrap();
    let i_le = bn_as_byte_array(&identity);
    let mut m_digest = [0u8; 20];
    m_digest[..i_le.len()].copy_from_slice(&i_le);
    m_digest.reverse();
    let mut hasher = Sha1::new();
    hasher.update(bn_as_byte_array(&salt));
    hasher.update(m_digest);
    let x = bn_from_binary(&hasher.finalize());

    let u_digest = {
        let mut hasher = Sha1::new();
        hasher.update(bn_as_byte_array(&big_a));
        hasher.update(bn_as_byte_array(&b_pub));
        hasher.finalize()
    };
    let u = bn_from_binary(&u_digest);
    // S_client = (B - 3*g^x)^(a + u*x) mod N
    let gx = g.modpow(&x, &n);
    let base = ((&b_pub + &n - (&gx * 3u32) % &n) % &n).clone();
    let s_client = base.modpow(&(&a + &u * &x), &n);
    let k_client = hash_session_key(&s_client);
    assert_eq!(server.session_key_bytes(), k_client);

    // Client M1 with the CalculateProof formula.
    let mut hash_n = Sha1::digest(bn_as_byte_array(&n));
    let hash_g = Sha1::digest(bn_as_byte_array(&g));
    for i in 0..20 {
        hash_n[i] ^= hash_g[i];
    }
    let t3 = bn_from_binary(&hash_n);
    let mut hasher = Sha1::new();
    hasher.update(bn_as_byte_array(&t3));
    hasher.update(Sha1::digest(username.as_bytes()));
    hasher.update(bn_as_byte_array(&salt));
    hasher.update(bn_as_byte_array(&big_a));
    hasher.update(bn_as_byte_array(&b_pub));
    hasher.update(bn_as_byte_array(&bn_from_binary(&k_client)));
    let m1: [u8; 20] = hasher.finalize().into();
    assert!(server.proof_matches(&m1));

    // Wrong password must not match.
    let mut wrong = SrpSession::host_ephemeral_with_b(
        verifier,
        salt,
        BigUint::parse_bytes(b"1234567890abcdef1234567890abcdef123456", 16).unwrap(),
    );
    assert!(wrong.calculate_session_key(&a_wire));
    wrong.calculate_proof(username);
    let mut bad_m1 = m1;
    bad_m1[0] ^= 0xFF;
    assert!(!wrong.proof_matches(&bad_m1));

    // M2 client check: SHA1(A || M || K) with the same feeds.
    let mut hasher = Sha1::new();
    hasher.update(bn_as_byte_array(&big_a));
    hasher.update(bn_as_byte_array(&bn_from_binary(&m1)));
    hasher.update(bn_as_byte_array(&bn_from_binary(&k_client)));
    let m2_expected: [u8; 20] = hasher.finalize().into();
    assert_eq!(server.finalize(), m2_expected);
}

#[test]
fn session_key_rejects_zero_a() {
    let salt = generate_salt();
    let verifier = calculate_verifier(&sha_pass_hash("U", "P"), &salt);
    let mut server = SrpSession::host_ephemeral(verifier, salt);
    assert!(!server.calculate_session_key(&[0u8; 32]));
}
