//! Auth/logon packet layouts (`vmangos/src/realmd/AuthPackets.h`,
//! `AuthCodes.h`) and the server→client builders whose byte order is
//! fixed by `AuthSocket.cpp`.

use crate::buffer::{PacketReader, PacketWriter, ReadError};

/// `eAuthCmd` (`AuthCodes.h:29-40`).
pub mod cmd {
    pub const AUTH_LOGON_CHALLENGE: u8 = 0x00;
    pub const AUTH_LOGON_PROOF: u8 = 0x01;
    pub const AUTH_RECONNECT_CHALLENGE: u8 = 0x02;
    pub const AUTH_RECONNECT_PROOF: u8 = 0x03;
    pub const REALM_LIST: u8 = 0x10;
    pub const XFER_INITIATE: u8 = 0x30;
    pub const XFER_DATA: u8 = 0x31;
    pub const XFER_ACCEPT: u8 = 0x32;
    pub const XFER_RESUME: u8 = 0x33;
    pub const XFER_CANCEL: u8 = 0x34;
}

/// `eAuthResult` subset realmd sends (`AuthCodes.h`). The full enum
/// is transcribed when the auth binary's error paths land; values
/// here are the ones the packet builders below emit.
pub mod result {
    pub const WOW_SUCCESS: u8 = 0x00;
    pub const WOW_FAIL_BANNED: u8 = 0x03;
    pub const WOW_FAIL_UNKNOWN_ACCOUNT: u8 = 0x04;
    pub const WOW_FAIL_INCORRECT_PASSWORD: u8 = 0x05;
    pub const WOW_FAIL_VERSION_INVALID: u8 = 0x09;
    pub const WOW_FAIL_SUSPENDED: u8 = 0x0C;
    pub const WOW_FAIL_FAIL_NOACCESS: u8 = 0x0D;
    pub const WOW_FAIL_DB_BUSY: u8 = 0x08;
    pub const WOW_FAIL_VERSION_UPDATE: u8 = 0x0A;
    pub const WOW_FAIL_PARENTCONTROL: u8 = 0x0F;
}

/// The static version-challenge bytes sent in the logon challenge
/// response and used as CRC salt in version checks
/// (`AuthSocket.cpp:61`).
pub const VERSION_CHALLENGE: [u8; 16] = [
    0xBA, 0xA3, 0x1E, 0x99, 0xA0, 0x0B, 0x21, 0x57, 0xFC, 0x37, 0x3F, 0xB3, 0x69, 0xCD, 0xD2, 0xF1,
];

/// `sAuthLogonChallengeHeader` — follows the 1-byte command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogonChallengeHeader {
    pub error: u8,
    pub size: u16,
}

/// `sAuthLogonChallengeBody` with the username decoded per
/// `username_len` (the wire body is variable-length; the fixed
/// struct's tail is the 17-byte username buffer).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogonChallengeBody {
    pub gamename: [u8; 4],
    pub version: (u8, u8, u8),
    pub build: u16,
    pub platform: [u8; 4],
    pub os: [u8; 4],
    pub country: [u8; 4],
    pub timezone_bias: u32,
    pub ip: u32,
    pub username: String,
}

/// Parses the 3-byte challenge header.
///
/// # Errors
/// [`ReadError`] on truncation.
pub fn parse_logon_challenge_header(
    reader: &mut PacketReader<'_>,
) -> Result<LogonChallengeHeader, ReadError> {
    Ok(LogonChallengeHeader {
        error: reader.read_u8()?,
        size: reader.read_u16_le()?,
    })
}

/// Parses a logon challenge body of exactly `header.size` bytes as
/// the caller sliced it.
///
/// # Errors
/// [`ReadError`] on truncation (username longer than the body is
/// clamped to what remains, mirroring the fixed-size struct read).
pub fn parse_logon_challenge_body(data: &[u8]) -> Result<LogonChallengeBody, ReadError> {
    let mut reader = PacketReader::new(data);
    let gamename = reader.read_array()?;
    let version = (reader.read_u8()?, reader.read_u8()?, reader.read_u8()?);
    let build = reader.read_u16_le()?;
    let platform = reader.read_array()?;
    let os = reader.read_array()?;
    let country = reader.read_array()?;
    let timezone_bias = reader.read_u32_le()?;
    let ip = reader.read_u32_le()?;
    let username_len = usize::from(reader.read_u8()?);
    let take = username_len.min(reader.remaining());
    let username = String::from_utf8_lossy(reader.read_bytes(take)?).into_owned();
    Ok(LogonChallengeBody {
        gamename,
        version,
        build,
        platform,
        os,
        country,
        timezone_bias,
        ip,
        username,
    })
}

/// `sAuthLogonProof_C` (+ optional PIN block). `security_flags` and
/// `pin` are present only for builds ≥ 5428, matching the C++ struct
/// selection (`AuthSocket.cpp:497`-ish / `m_build < 5428` skip).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogonProof {
    pub a: [u8; 32],
    pub m1: [u8; 20],
    pub crc_hash: [u8; 20],
    pub number_of_keys: u8,
    pub security_flags: u8,
    pub pin_salt: Option<[u8; 16]>,
    pub pin_hash: Option<[u8; 20]>,
}

/// Parses a logon proof body (after the command byte).
///
/// # Errors
/// [`ReadError`] on truncation.
pub fn parse_logon_proof(data: &[u8], build: u16) -> Result<LogonProof, ReadError> {
    let mut reader = PacketReader::new(data);
    let a = reader.read_array()?;
    let m1 = reader.read_array()?;
    let crc_hash = reader.read_array()?;
    let number_of_keys = reader.read_u8()?;
    let mut proof = LogonProof {
        a,
        m1,
        crc_hash,
        number_of_keys,
        security_flags: 0,
        pin_salt: None,
        pin_hash: None,
    };
    if build >= 5428 {
        proof.security_flags = reader.read_u8()?;
        if proof.security_flags & 0x01 != 0 {
            proof.pin_salt = Some(reader.read_array()?);
            proof.pin_hash = Some(reader.read_array()?);
        }
    }
    Ok(proof)
}

/// `AUTH_RECONNECT_PROOF_C`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconnectProof {
    pub r1: [u8; 16],
    pub r2: [u8; 20],
    pub r3: [u8; 20],
    pub number_of_keys: u8,
}

/// Parses a reconnect proof body (after the command byte).
///
/// # Errors
/// [`ReadError`] on truncation.
pub fn parse_reconnect_proof(data: &[u8]) -> Result<ReconnectProof, ReadError> {
    let mut reader = PacketReader::new(data);
    Ok(ReconnectProof {
        r1: reader.read_array()?,
        r2: reader.read_array()?,
        r3: reader.read_array()?,
        number_of_keys: reader.read_u8()?,
    })
}

/// The optional PIN-grid tail of a challenge response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PinGrid {
    /// Wire form is endian-swapped (`EndianConvert`, `AuthSocket.cpp:429`).
    pub grid_seed_be: u32,
    pub salt: [u8; 16],
}

/// Builds the successful logon challenge response bytes:
/// cmd, 0x00, `WOW_SUCCESS`, B(32, LE-padded), `g_len`(1), g, `N_len`(32),
/// N(32, LE-padded), salt (minimal LE), version challenge(16), then
/// the PIN tail or — for build ≥ 5428 without PIN — a single 0x00.
/// The leading 0x00 is written unconditionally by the C++ before the
/// result byte (`AuthSocket.cpp:280-281,405`). g and N are the SRP
/// constants passed pre-encoded to keep this builder crypto-free.
#[must_use]
pub fn build_logon_challenge_success(
    b_bytes32: &[u8],
    g_bytes: &[u8],
    n_bytes32: &[u8],
    salt_bytes: &[u8],
    build: u16,
    pin: Option<PinGrid>,
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.write_u8(cmd::AUTH_LOGON_CHALLENGE)
        .write_u8(0)
        .write_u8(result::WOW_SUCCESS)
        .write_bytes(b_bytes32)
        .write_u8(1)
        .write_bytes(g_bytes)
        .write_u8(32)
        .write_bytes(n_bytes32)
        .write_bytes(salt_bytes)
        .write_bytes(&VERSION_CHALLENGE);
    if let Some(pin) = pin {
        w.write_u8(1)
            .write_u32_le(pin.grid_seed_be)
            .write_bytes(&pin.salt);
    } else if build >= 5428 {
        w.write_u8(0);
    }
    w.into_vec()
}

/// Builds a bare failure: cmd + error. This 2-byte shape is what the
/// proof-stage handlers send (`CMD_AUTH_LOGON_PROOF` /
/// `CMD_AUTH_RECONNECT_PROOF` paths); challenge-stage failures are
/// 3 bytes — see [`build_challenge_failure`].
#[must_use]
pub fn build_logon_failure(auth_cmd: u8, error: u8) -> Vec<u8> {
    vec![auth_cmd, error]
}

/// Builds a challenge-stage failure: `CMD_AUTH_LOGON_CHALLENGE`, 0x00,
/// error — the shape `_HandleLogonChallenge` assembles incrementally
/// (`AuthSocket.cpp:280-281` + the per-branch result byte).
#[must_use]
pub fn build_challenge_failure(error: u8) -> Vec<u8> {
    vec![cmd::AUTH_LOGON_CHALLENGE, 0, error]
}

/// Builds the logon proof success response for the client's build
/// (`GenerateLogonProofResponse`): build < 6299 →
/// `AUTH_LOGON_PROOF_S` (cmd, 0, M2, surveyId); < 8089 adds
/// loginFlags; ≥ 8089 adds accountFlags = PROPASS. 1.12.1 (5875)
/// always takes the first form.
#[must_use]
pub fn build_logon_proof_success(build: u16, m2: &[u8; 20]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.write_u8(cmd::AUTH_LOGON_PROOF)
        .write_u8(0)
        .write_bytes(m2);
    if build < 6299 {
        w.write_u32_le(0);
    } else if build < 8089 {
        w.write_u32_le(0).write_u16_le(0);
    } else {
        w.write_u32_le(ACCOUNT_FLAG_PROPASS)
            .write_u32_le(0)
            .write_u16_le(0);
    }
    w.into_vec()
}

/// `ACCOUNT_FLAG_PROPASS` (`AuthCodes.h:118`).
pub const ACCOUNT_FLAG_PROPASS: u32 = 0x0040_0000;
