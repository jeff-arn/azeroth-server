//! World-server wire core for build 5875, byte-faithful to
//! `WorldSocket.{h,cpp}`, `WorldPacket.h`, `AuthCrypt.{h,cpp}`,
//! `Opcodes.h`, and `PacketProcessing.h`.
//!
//! This module intentionally owns only transport/session-entry
//! primitives. Handler dispatch and world simulation belong to the
//! future `mangosd` runtime; the generated opcode registry supplies
//! the metadata those layers share.

use num_bigint::BigUint;
use sha1::{Digest, Sha1};

use crate::buffer::{PacketReader, PacketWriter, ReadError};
use crate::srp::{bn_as_byte_array, bn_from_hex};

/// Number of opcode slots in the 1.12.1 table
/// (`Opcodes_1_12_1.h::NUM_MSG_TYPES`). There are 826 registered
/// rows because two slots are deliberately unregistered.
pub const NUM_MSG_TYPES: usize = 828;

/// Encrypted client packet header size on the wire.
pub const CLIENT_HEADER_SIZE: usize = 6;
/// Server packet header size on the wire.
pub const SERVER_HEADER_SIZE: usize = 4;
/// Largest accepted `ClientPktHeader::size` value
/// (`WorldSocket.cpp:111`; includes the 4-byte command).
pub const MAX_CLIENT_PACKET_SIZE: u16 = 0x2800;
/// Largest payload representable by the u16 server size field after
/// accounting for its 2-byte command.
pub const MAX_SERVER_PAYLOAD_SIZE: usize = (u16::MAX - 2) as usize;
/// Outbound queue high-water mark after which C++ disconnects the
/// socket (`WorldSocket.cpp:523`).
pub const SEND_QUEUE_CAPACITY: usize = 1024;

/// First-slice world opcodes. The complete generated registry lives
/// in [`crate::opcode_table`].
pub mod opcode {
    pub const CMSG_CHAR_ENUM: u16 = 55;
    pub const SMSG_CHAR_ENUM: u16 = 59;
    pub const CMSG_PLAYER_LOGIN: u16 = 61;
    pub const CMSG_PING: u16 = 476;
    pub const SMSG_PONG: u16 = 477;
    pub const SMSG_AUTH_CHALLENGE: u16 = 492;
    pub const CMSG_AUTH_SESSION: u16 = 493;
    pub const SMSG_AUTH_RESPONSE: u16 = 494;
    pub const SMSG_LOGIN_VERIFY_WORLD: u16 = 566;
}

/// `SMSG_AUTH_RESPONSE` result bytes from `SharedDefines.h` (the
/// shared logon/world response enum; `AUTH_OK == 12`).
pub mod auth_response {
    pub const OK: u8 = 12;
    pub const FAILED: u8 = 13;
    pub const REJECT: u8 = 14;
    pub const BAD_SERVER_PROOF: u8 = 15;
    pub const UNAVAILABLE: u8 = 16;
    pub const SYSTEM_ERROR: u8 = 17;
    pub const BILLING_ERROR: u8 = 18;
    pub const BILLING_EXPIRED: u8 = 19;
    pub const VERSION_MISMATCH: u8 = 20;
    pub const UNKNOWN_ACCOUNT: u8 = 21;
    pub const INCORRECT_PASSWORD: u8 = 22;
    pub const SESSION_EXPIRED: u8 = 23;
    pub const SERVER_SHUTTING_DOWN: u8 = 24;
    pub const ALREADY_LOGGING_IN: u8 = 25;
    pub const LOGIN_SERVER_NOT_FOUND: u8 = 26;
    pub const WAIT_QUEUE: u8 = 27;
    pub const BANNED: u8 = 28;
    pub const ALREADY_ONLINE: u8 = 29;
    pub const NO_TIME: u8 = 30;
    pub const DB_BUSY: u8 = 31;
    pub const SUSPENDED: u8 = 32;
    pub const PARENTAL_CONTROL: u8 = 33;
}

/// `SessionStatus` (`Opcodes.h:140-148`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum SessionStatus {
    Authed = 0,
    LoggedIn = 1,
    Transfer = 2,
    LoggedInOrRecentlyLoggedOut = 3,
    Never = 4,
    Unhandled = 5,
}

/// Canonical `PacketProcessing` queue classes
/// (`PacketProcessing.h:25-76`). C++ aliases are associated
/// constants below because several names intentionally share a
/// queue class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum PacketProcessing {
    World = 0,
    Map = 1,
    Spells = 2,
    Movement = 3,
    Async = 4,
    MaxType = 5,
}

impl PacketProcessing {
    pub const SELF_ITEMS: Self = Self::Map;
    pub const DB_QUERY: Self = Self::Async;
    pub const CHANNEL: Self = Self::World;
    pub const GROUP: Self = Self::World;
    pub const GUILD: Self = Self::World;
}

/// Header-only rolling XOR used by the world socket
/// (`AuthCrypt.cpp:29-74`). State indices are u8 exactly as in C++;
/// each byte first reduces its index modulo the key length.
#[derive(Clone, Debug, Default)]
pub struct AuthCrypt {
    key: Vec<u8>,
    send_i: u8,
    send_j: u8,
    recv_i: u8,
    recv_j: u8,
    initialized: bool,
}

impl AuthCrypt {
    /// Bytes of each received client header that are encrypted.
    pub const CRYPTED_RECV_LEN: usize = 6;
    /// Bytes of each sent server header that are encrypted.
    pub const CRYPTED_SEND_LEN: usize = 4;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `Init`: reset both directional indices. The key is left as
    /// previously set, matching the C++ call order (`SetKey` then
    /// `Init` in `_HandleAuthSession`).
    pub fn init(&mut self) {
        self.send_i = 0;
        self.send_j = 0;
        self.recv_i = 0;
        self.recv_j = 0;
        self.initialized = true;
    }

    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// `SetKey`: C++ replaces an empty key with one zero byte so
    /// the modulo in the transform is defined.
    pub fn set_key(&mut self, key: &[u8]) {
        self.key = if key.is_empty() {
            vec![0]
        } else {
            key.to_vec()
        };
    }

    /// Set the key from an SRP session key using the same minimal
    /// little-endian feed as `K.AsByteArray()`.
    pub fn set_key_biguint(&mut self, key: &BigUint) {
        self.set_key(&bn_as_byte_array(key));
    }

    fn next_recv_index(&mut self) -> usize {
        self.recv_i = (usize::from(self.recv_i) % self.key.len()) as u8;
        let index = usize::from(self.recv_i);
        self.recv_i = self.recv_i.wrapping_add(1);
        index
    }

    fn next_send_index(&mut self) -> usize {
        self.send_i = (usize::from(self.send_i) % self.key.len()) as u8;
        let index = usize::from(self.send_i);
        self.send_i = self.send_i.wrapping_add(1);
        index
    }

    /// `DecryptRecv`: transforms only the first six bytes and is a
    /// no-op before `init` (or, defensively, with no key set).
    /// C++ has undefined modulo-by-zero behaviour in the latter,
    /// unreachable state; Rust leaves the bytes unchanged instead
    /// of panicking.
    pub fn decrypt_recv(&mut self, data: &mut [u8]) {
        if !self.initialized || self.key.is_empty() || data.len() < Self::CRYPTED_RECV_LEN {
            return;
        }
        for byte in data.iter_mut().take(Self::CRYPTED_RECV_LEN) {
            let cipher = *byte;
            let index = self.next_recv_index();
            let key_byte = self.key[index];
            *byte = cipher.wrapping_sub(self.recv_j) ^ key_byte;
            self.recv_j = cipher;
        }
    }

    /// `EncryptSend`: transforms only the first four bytes and is
    /// a no-op before `init` (or, defensively, with no key set).
    pub fn encrypt_send(&mut self, data: &mut [u8]) {
        if !self.initialized || self.key.is_empty() || data.len() < Self::CRYPTED_SEND_LEN {
            return;
        }
        for byte in data.iter_mut().take(Self::CRYPTED_SEND_LEN) {
            let index = self.next_send_index();
            let key_byte = self.key[index];
            let cipher = (*byte ^ key_byte).wrapping_add(self.send_j);
            *byte = cipher;
            self.send_j = cipher;
        }
    }
}

/// A rejected client packet header. C++ logs and stops arming the
/// read loop on these conditions (`WorldSocket.cpp:111-119`);
/// returning the decoded values lets the Rust socket layer log the
/// same facts before applying its explicit close policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MalformedHeader {
    SizeTooSmall { size: u16, opcode: u32 },
    SizeTooLarge { size: u16, opcode: u32 },
    OpcodeOutOfRange { size: u16, opcode: u32 },
}

impl MalformedHeader {
    /// Decoded size, for the C++ malformed-packet log line.
    #[must_use]
    pub const fn size(&self) -> u16 {
        match self {
            Self::SizeTooSmall { size, .. }
            | Self::SizeTooLarge { size, .. }
            | Self::OpcodeOutOfRange { size, .. } => *size,
        }
    }

    /// Decoded opcode, for the C++ malformed-packet log line.
    #[must_use]
    pub const fn opcode(&self) -> u32 {
        match self {
            Self::SizeTooSmall { opcode, .. }
            | Self::SizeTooLarge { opcode, .. }
            | Self::OpcodeOutOfRange { opcode, .. } => *opcode,
        }
    }
}

impl std::fmt::Display for MalformedHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SizeTooSmall { size, opcode } => {
                write!(
                    f,
                    "client packet size {size} below minimum for opcode {opcode}"
                )
            }
            Self::SizeTooLarge { size, opcode } => {
                write!(
                    f,
                    "client packet size {size} above maximum for opcode {opcode}"
                )
            }
            Self::OpcodeOutOfRange { size, opcode } => {
                write!(f, "client opcode {opcode} out of range at size {size}")
            }
        }
    }
}

impl std::error::Error for MalformedHeader {}

/// Decoded `ClientPktHeader`: big-endian size (including the
/// 4-byte command, excluding the size field) and little-endian
/// command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientHeader {
    pub size: u16,
    pub opcode: u32,
}

impl ClientHeader {
    /// Decrypt and validate one six-byte wire header. As in C++,
    /// the crypt state advances even when validation then fails.
    ///
    /// # Errors
    ///
    /// Returns the decoded size/opcode in [`MalformedHeader`] when
    /// `size < 4`, `size > 0x2800`, or `opcode >= 828`.
    pub fn decode(
        crypt: &mut AuthCrypt,
        wire: &mut [u8; CLIENT_HEADER_SIZE],
    ) -> Result<Self, MalformedHeader> {
        crypt.decrypt_recv(wire);
        let header = Self {
            size: u16::from_be_bytes([wire[0], wire[1]]),
            opcode: u32::from_le_bytes([wire[2], wire[3], wire[4], wire[5]]),
        };
        if header.size < 4 {
            return Err(MalformedHeader::SizeTooSmall {
                size: header.size,
                opcode: header.opcode,
            });
        }
        if header.size > MAX_CLIENT_PACKET_SIZE {
            return Err(MalformedHeader::SizeTooLarge {
                size: header.size,
                opcode: header.opcode,
            });
        }
        if header.opcode as usize >= NUM_MSG_TYPES {
            return Err(MalformedHeader::OpcodeOutOfRange {
                size: header.size,
                opcode: header.opcode,
            });
        }
        Ok(header)
    }

    /// Payload byte count after the six-byte client header.
    #[must_use]
    pub const fn payload_len(self) -> usize {
        (self.size - 4) as usize
    }
}

/// A server payload too large for the frozen u16 header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketTooLarge {
    pub payload_len: usize,
}

impl std::fmt::Display for PacketTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "server payload length {} exceeds header capacity",
            self.payload_len
        )
    }
}

impl std::error::Error for PacketTooLarge {}

/// Encode and encrypt `ServerPktHeader` (`WorldSocket.cpp:574-582`).
///
/// # Errors
///
/// Returns [`PacketTooLarge`] when C++ would truncate the u16
/// size field; rejecting avoids silently emitting a corrupt frame.
pub fn encode_server_header(
    crypt: &mut AuthCrypt,
    opcode: u16,
    payload_len: usize,
) -> Result<[u8; SERVER_HEADER_SIZE], PacketTooLarge> {
    if payload_len > MAX_SERVER_PAYLOAD_SIZE {
        return Err(PacketTooLarge { payload_len });
    }
    let size = (payload_len + 2) as u16;
    let mut header = [
        size.to_be_bytes()[0],
        size.to_be_bytes()[1],
        opcode.to_le_bytes()[0],
        opcode.to_le_bytes()[1],
    ];
    crypt.encrypt_send(&mut header);
    Ok(header)
}

/// Owned world packet (`WorldPacket`): an opcode, a little-endian
/// `ByteBuffer` payload, and a receive timestamp in milliseconds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldPacket {
    opcode: u16,
    body: Vec<u8>,
    received_ms: u32,
}

impl WorldPacket {
    #[must_use]
    pub fn new(opcode: u16) -> Self {
        Self {
            opcode,
            body: Vec::new(),
            received_ms: 0,
        }
    }

    #[must_use]
    pub fn with_body(opcode: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            opcode,
            body: body.into(),
            received_ms: 0,
        }
    }

    #[must_use]
    pub fn received(opcode: u16, body: impl Into<Vec<u8>>, received_ms: u32) -> Self {
        Self {
            opcode,
            body: body.into(),
            received_ms,
        }
    }

    /// `Initialize`: clear the payload, set opcode, and reserve at
    /// least `reserve` bytes. The receive stamp is unchanged, as in
    /// `WorldPacket::Initialize`.
    pub fn initialize(&mut self, opcode: u16, reserve: usize) {
        self.body.clear();
        if self.body.capacity() < reserve {
            self.body.reserve(reserve - self.body.capacity());
        }
        self.opcode = opcode;
    }

    #[must_use]
    pub const fn opcode(&self) -> u16 {
        self.opcode
    }

    /// Raw access for explicitly hand-framed payloads. Normal code
    /// should use builders, [`WorldPacket::take_writer`], and
    /// [`WorldPacket::reader`].
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Raw mutable access for an existing caller-owned packet
    /// builder (for example spline fragments written in place).
    pub fn body_mut(&mut self) -> &mut Vec<u8> {
        &mut self.body
    }

    /// Consume a byte writer into this packet body.
    pub fn take_writer(&mut self, writer: PacketWriter) {
        self.body = writer.into_vec();
    }

    #[must_use]
    pub fn reader(&self) -> PacketReader<'_> {
        PacketReader::new(&self.body)
    }

    #[must_use]
    pub const fn received_ms(&self) -> u32 {
        self.received_ms
    }

    pub fn set_received_ms(&mut self, received_ms: u32) {
        self.received_ms = received_ms;
    }

    /// Encode the complete server wire frame. Only the header is
    /// encrypted; payload bytes are appended verbatim.
    ///
    /// # Errors
    ///
    /// Returns [`PacketTooLarge`] rather than reproducing C++'s
    /// truncating `static_cast<uint16>` size conversion.
    pub fn encode(&self, crypt: &mut AuthCrypt) -> Result<Vec<u8>, PacketTooLarge> {
        let header = encode_server_header(crypt, self.opcode, self.body.len())?;
        let mut out = Vec::with_capacity(SERVER_HEADER_SIZE + self.body.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&self.body);
        Ok(out)
    }
}

/// `SMSG_AUTH_CHALLENGE << authSeed` (`WorldSocket.cpp:508-510`).
#[must_use]
pub fn build_auth_challenge(auth_seed: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(auth_seed);
    let mut packet = WorldPacket::new(opcode::SMSG_AUTH_CHALLENGE);
    packet.take_writer(writer);
    packet
}

/// Single-byte `SMSG_AUTH_RESPONSE` used by every world-auth exit
/// in `WorldSocket::_HandleAuthSession`.
#[must_use]
pub fn build_auth_response(code: u8) -> WorldPacket {
    WorldPacket::with_body(opcode::SMSG_AUTH_RESPONSE, [code])
}

/// Parsed `CMSG_PING` body at the supported 1.12.1 build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingPacket {
    pub ping: u32,
    pub latency: u32,
}

/// Parse `CMSG_PING`: two little-endian u32 values
/// (`WorldSocket.cpp:444-453` under build > 1.8.4).
///
/// # Errors
///
/// Returns [`ReadError`] on a short body instead of reproducing
/// C++'s exception path as a panic.
pub fn parse_ping(body: &[u8]) -> Result<PingPacket, ReadError> {
    let mut reader = PacketReader::new(body);
    Ok(PingPacket {
        ping: reader.read_u32_le()?,
        latency: reader.read_u32_le()?,
    })
}

/// `SMSG_PONG << ping` (`WorldSocket.cpp:498-500`).
#[must_use]
pub fn build_pong(ping: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(ping);
    let mut packet = WorldPacket::new(opcode::SMSG_PONG);
    packet.take_writer(writer);
    packet
}

/// Parsed `CMSG_AUTH_SESSION` body. Bytes after the digest are
/// retained verbatim for `WorldSession::BuildAddonPacket`, which in
/// C++ continues reading the same `WorldPacket`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthSessionPacket {
    pub client_build: u32,
    pub server_id: u32,
    pub account: String,
    pub client_seed: u32,
    pub digest: [u8; 20],
    pub addon_data: Vec<u8>,
}

/// Parse `CMSG_AUTH_SESSION` in `_HandleAuthSession` order:
/// build, server id, account C-string, client seed, SHA1 digest.
/// The account bytes use the existing lossy C-string reader like
/// the logon protocol; database lookup remains the authority on
/// whether the resulting account name exists.
///
/// # Errors
///
/// Returns [`ReadError`] if any fixed field is absent. A missing
/// account terminator consumes the remaining input and therefore
/// also fails on the subsequent fixed field.
pub fn parse_auth_session(body: &[u8]) -> Result<AuthSessionPacket, ReadError> {
    let mut reader = PacketReader::new(body);
    let client_build = reader.read_u32_le()?;
    let server_id = reader.read_u32_le()?;
    let account = reader.read_cstring();
    let client_seed = reader.read_u32_le()?;
    let digest = reader.read_array::<20>()?;
    let addon_data = reader.read_bytes(reader.remaining())?.to_vec();
    Ok(AuthSessionPacket {
        client_build,
        server_id,
        account,
        client_seed,
        digest,
        addon_data,
    })
}

/// Parse a stored `account.sessionkey` hex value into the SRP
/// session key used by world auth and `AuthCrypt`.
#[must_use]
pub fn session_key_from_hex(hex: &str) -> Option<BigUint> {
    bn_from_hex(hex)
}

/// Expected world-auth digest from `WorldSocket.cpp:353-364`:
/// SHA1(account || zero-u32 || client-seed || server-seed || K),
/// with integer words in supported little-endian host order and K
/// in `BigNumber::AsByteArray()` minimal little-endian form.
#[must_use]
pub fn expected_auth_session_digest(
    account: &str,
    client_seed: u32,
    server_seed: u32,
    session_key: &BigUint,
) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(account.as_bytes());
    hasher.update(0u32.to_le_bytes());
    hasher.update(client_seed.to_le_bytes());
    hasher.update(server_seed.to_le_bytes());
    hasher.update(bn_as_byte_array(session_key));
    hasher.finalize().into()
}

/// Verify a parsed packet's digest. This deliberately returns a
/// boolean instead of exposing which input differed.
#[must_use]
pub fn auth_session_digest_matches(
    packet: &AuthSessionPacket,
    server_seed: u32,
    session_key: &BigUint,
) -> bool {
    packet.digest
        == expected_auth_session_digest(
            &packet.account,
            packet.client_seed,
            server_seed,
            session_key,
        )
}
