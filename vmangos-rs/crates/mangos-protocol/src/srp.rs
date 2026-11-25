//! SRP6 server math, byte-faithful to
//! `vmangos/src/shared/Crypto/Authentication/SRP6.cpp`.
//!
//! Every `BigNumber` SHA1 feed in C++ goes through
//! `AsByteArray(minSize = 0, reverse = true)` (`BigNumber.h:87`):
//! minimal-length little-endian, empty for zero. The helpers here
//! reproduce that exactly; see `PHASE1-PROGRESS.md` for the two
//! characterized traps (the inverted `SRP6::Proof`, cancelled by its
//! caller, and the minimal-length salt feed). This module exposes
//! sane semantics (`proof_matches` returns true on a match); the
//! observed wire behaviour is identical to the C++ oracle.

use num_bigint::BigUint;
use sha1::{Digest, Sha1};

/// The SRP prime N (`SRP6.cpp:31`).
pub const N_HEX: &str = "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7";
/// The generator g (`SRP6.cpp:32`).
pub const G: u32 = 7;
/// Salt size in bytes (`s_BYTE_SIZE`, `SRP6.h:29`).
pub const SALT_BYTES: usize = 32;

/// The prime N as an integer.
///
/// # Panics
/// Never in practice: the compile-time constant [`N_HEX`] is valid
/// hex. The `expect` documents that invariant.
#[must_use]
pub fn prime_n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("constant N parses")
}

/// `BigNumber::AsByteArray(0, true)`: minimal little-endian, empty
/// for zero.
#[must_use]
pub fn bn_as_byte_array(value: &BigUint) -> Vec<u8> {
    value.to_bytes_le()
}

/// `BigNumber::AsByteArray(min_size, true)`: little-endian,
/// zero-extended at the high end to at least `min_size` bytes.
#[must_use]
pub fn bn_as_byte_array_padded(value: &BigUint, min_size: usize) -> Vec<u8> {
    let mut bytes = value.to_bytes_le();
    if bytes.len() < min_size {
        bytes.resize(min_size, 0);
    }
    bytes
}

/// `BigNumber::SetBinary`: little-endian bytes to integer.
#[must_use]
pub fn bn_from_binary(bytes: &[u8]) -> BigUint {
    BigUint::from_bytes_le(bytes)
}

/// `BigNumber::AsHexStr` (`BN_bn2hex`): uppercase, unpadded.
#[must_use]
pub fn bn_to_hex(value: &BigUint) -> String {
    value.to_str_radix(16).to_uppercase()
}

/// `BigNumber::SetHexStr`: parse hex (either case). Returns `None`
/// where C++ would fail (`BN_hex2bn` error or a zero result rejected
/// by `SetSalt`/`SetVerifier` guards is the caller's concern).
#[must_use]
pub fn bn_from_hex(hex: &str) -> Option<BigUint> {
    BigUint::parse_bytes(hex.as_bytes(), 16)
}

fn sha1_of(parts: &[&[u8]]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn feed(value: &BigUint) -> Vec<u8> {
    bn_as_byte_array(value)
}

/// `CalculateShaPassHash` + `normalizeString`
/// (`AccountMgr.cpp`, `dashboard/auth.py:69-72`): uppercase hex of
/// SHA1("USERNAME:PASSWORD") with both parts uppercased first.
#[must_use]
pub fn sha_pass_hash(username: &str, password: &str) -> String {
    let digest = Sha1::digest(format!(
        "{}:{}",
        username.to_uppercase(),
        password.to_uppercase()
    ));
    let mut out = String::with_capacity(40);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02X}");
    }
    out
}

/// A fresh salt with the `BN_rand(_bn, 256, top = 0, bottom = 1)`
/// properties (`BigNumber.cpp:55`): bit 255 set, odd — always exactly
/// 32 minimal bytes.
#[must_use]
pub fn generate_salt() -> BigUint {
    let mut bytes = [0u8; SALT_BYTES];
    rand::fill(&mut bytes);
    bytes[0] |= 0x80;
    bytes[SALT_BYTES - 1] |= 0x01;
    BigUint::from_bytes_be(&bytes)
}

/// `SRP6::CalculateVerifier(rI, salt)` (`SRP6.cpp:97-125`):
/// `v = g^x mod N` with
/// `x = SHA1(A_empty || s_minLE || mDigest)` as a little-endian
/// integer, where `mDigest` is the identity hash's minimal LE bytes
/// placed at the front of a zeroed 20-byte buffer, then reversed.
/// The `A` feed is the zero `BigNumber` at every C++ call site and
/// contributes zero bytes (Phase 0/1 characterization).
#[must_use]
pub fn calculate_verifier(sha_pass_hex: &str, salt: &BigUint) -> BigUint {
    let identity = bn_from_hex(sha_pass_hex).unwrap_or_default();
    let i_bytes = bn_as_byte_array(&identity);
    let mut m_digest = [0u8; 20];
    if i_bytes.len() <= 20 {
        m_digest[..i_bytes.len()].copy_from_slice(&i_bytes);
    }
    m_digest.reverse();
    let empty: [u8; 0] = [];
    let salt_feed = feed(salt);
    let digest = sha1_of(&[&empty, &salt_feed, &m_digest]);
    let x = bn_from_binary(&digest);
    BigUint::from(G).modpow(&x, &prime_n())
}

/// Server-side SRP6 session state (the member set of C++ `SRP6`
/// that the logon flow uses).
#[derive(Clone, Debug)]
pub struct SrpSession {
    b: BigUint,
    public_b: BigUint,
    v: BigUint,
    s: BigUint,
    a: BigUint,
    k: [u8; 40],
    m: [u8; 20],
}

impl SrpSession {
    /// `CalculateHostPublicEphemeral` (`SRP6.cpp:35-42`) with a fresh
    /// random 19-byte `b`: `B = (3v + g^b) mod N`.
    #[must_use]
    pub fn host_ephemeral(verifier: BigUint, salt: BigUint) -> Self {
        let mut b_bytes = [0u8; 19];
        rand::fill(&mut b_bytes);
        // BN_rand(bits = 152, top = -1 default in SetRand call is
        // BN_rand(_bn, numbits, 0, 1) via SetRand(19*8): top bit set.
        b_bytes[0] |= 0x80;
        b_bytes[18] |= 0x01;
        Self::host_ephemeral_with_b(verifier, salt, BigUint::from_bytes_be(&b_bytes))
    }

    /// Same with an injected `b` (deterministic tests / oracle
    /// differential runs).
    #[must_use]
    pub fn host_ephemeral_with_b(verifier: BigUint, salt: BigUint, b: BigUint) -> Self {
        let n = prime_n();
        let gmod = BigUint::from(G).modpow(&b, &n);
        let public_b = ((&verifier * 3u32) + &gmod) % &n;
        Self {
            b,
            public_b,
            v: verifier,
            s: salt,
            a: BigUint::ZERO,
            k: [0u8; 40],
            m: [0u8; 20],
        }
    }

    /// B on the wire: `AsByteArray(32)` (`AuthSocket.cpp:408`).
    #[must_use]
    pub fn public_b_bytes32(&self) -> Vec<u8> {
        bn_as_byte_array_padded(&self.public_b, 32)
    }

    /// Salt on the wire: `AsByteArray()` minimal (`AuthSocket.cpp:413`).
    #[must_use]
    pub fn salt_bytes(&self) -> Vec<u8> {
        bn_as_byte_array(&self.s)
    }

    /// `CalculateSessionKey` + `HashSessionKey` (`SRP6.cpp:69-88`,
    /// `127-154`). Returns false where C++ aborts (A == 0 or
    /// A mod N == 0).
    pub fn calculate_session_key(&mut self, a_bytes: &[u8]) -> bool {
        let n = prime_n();
        self.a = bn_from_binary(a_bytes);
        if self.a == BigUint::ZERO || (&self.a % &n) == BigUint::ZERO {
            return false;
        }
        let a_feed = feed(&self.a);
        let b_feed = feed(&self.public_b);
        let u = bn_from_binary(&sha1_of(&[&a_feed, &b_feed]));
        let s_key = ((&self.a * self.v.modpow(&u, &n)) % &n).modpow(&self.b, &n);
        self.k = hash_session_key(&s_key);
        true
    }

    /// `CalculateProof` (`SRP6.cpp:44-67`): computes the expected
    /// client proof M for `username` (as-sent case; C++ hashes the
    /// `m_login` string as stored).
    pub fn calculate_proof(&mut self, username: &str) {
        let n = prime_n();
        let mut hash_n = sha1_of(&[&feed(&n)]);
        let hash_g = sha1_of(&[&feed(&BigUint::from(G))]);
        for i in 0..20 {
            hash_n[i] ^= hash_g[i];
        }
        let t3 = bn_from_binary(&hash_n);
        let user_hash = Sha1::digest(username.as_bytes());
        self.m = sha1_of(&[
            &feed(&t3),
            &user_hash,
            &feed(&self.s),
            &feed(&self.a),
            &feed(&self.public_b),
            &feed(&bn_from_binary(&self.k)),
        ]);
    }

    /// Observed C++ wire semantics: true iff the client's M1 equals
    /// the computed M. (C++ expresses this as an inverted `Proof`
    /// negated by the caller — see crate docs.)
    #[must_use]
    pub fn proof_matches(&self, client_m1: &[u8; 20]) -> bool {
        &self.m == client_m1
    }

    /// `Finalize` (M2, `SRP6.cpp:168-175`): `SHA1(A || M || K)` with
    /// `BigNumber` feeds, M/K re-interpreted via `SetBinary` in C++ —
    /// equivalently their minimal-LE encodings.
    #[must_use]
    pub fn finalize(&self) -> [u8; 20] {
        sha1_of(&[
            &feed(&self.a),
            &feed(&bn_from_binary(&self.m)),
            &feed(&bn_from_binary(&self.k)),
        ])
    }

    /// The 40-byte strong session key K (interleaved form).
    #[must_use]
    pub fn session_key_bytes(&self) -> [u8; 40] {
        self.k
    }

    /// K as stored in `account.sessionkey`: `AsHexStr` of the
    /// `BigNumber` formed by `SetBinary(vK, 40)` — uppercase hex of the
    /// little-endian value, unpadded.
    #[must_use]
    pub fn session_key_hex(&self) -> String {
        bn_to_hex(&bn_from_binary(&self.k))
    }
}

/// `HashSessionKey` (`SRP6.cpp:127-154`): de-interleave the 32-byte
/// little-endian S, SHA1 each half, re-interleave into 40-byte K.
#[must_use]
pub fn hash_session_key(s_value: &BigUint) -> [u8; 40] {
    let t = bn_as_byte_array_padded(s_value, 32);
    let mut even = [0u8; 16];
    let mut odd = [0u8; 16];
    for i in 0..16 {
        even[i] = t[i * 2];
        odd[i] = t[i * 2 + 1];
    }
    let hash_even = sha1_of(&[&even]);
    let hash_odd = sha1_of(&[&odd]);
    let mut k = [0u8; 40];
    for i in 0..20 {
        k[i * 2] = hash_even[i];
        k[i * 2 + 1] = hash_odd[i];
    }
    k
}

/// Reconnect proof verification (`AuthSocket.cpp:883`): true iff
/// `SHA1(login || t1 || proof || K)` (`BigNumber` feeds after the login
/// string) equals R2.
#[must_use]
pub fn verify_reconnect_proof(
    login: &str,
    r1: &[u8; 16],
    reconnect_proof: &BigUint,
    strong_key: &BigUint,
    r2: &[u8; 20],
) -> bool {
    let t1 = bn_from_binary(r1);
    let digest = sha1_of(&[
        login.as_bytes(),
        &feed(&t1),
        &feed(reconnect_proof),
        &feed(strong_key),
    ]);
    &digest == r2
}
