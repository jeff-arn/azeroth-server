//! Scanner/fuzz robustness: malformed input must close (or be
//! ignored per the C++ bad-packet policy) without crashing the
//! socket task or the world actor, and a valid login must still
//! complete afterwards. Randomness is a fixed-seed xorshift —
//! deterministic by design.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration, timeout};

use mangos_db::DbError;
use mangos_db::characters::{AccountDataEntry, CharEnumRow};
use mangos_log::Log;
use mangos_protocol::world::{
    auth_response, expected_auth_session_digest, opcode, session_key_from_hex,
};
use mangos_world_data::enum_stores::EnumStores;

use mangosd::ServerConfig;
use mangosd::auth::{AuthBackend, WorldAuthAccount};
use mangosd::char_queries::CharQueries;
use mangosd::socket::{ConnectionDeps, run_connection};
use mangosd::world_actor::{RealmPublisher, SessionId, WorldDeps, spawn_world};

const SESSIONKEY_HEX: &str = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";
const CLIENT_SEED: u32 = 0x1122_3344;

// --- fakes (minimal; the loopback suite covers behaviour) ----

#[derive(Default)]
struct FakeBackend {
    writes: Mutex<Vec<(String, String)>>,
}

impl AuthBackend for FakeBackend {
    fn lookup_world_account<'a>(
        &'a self,
        _realm_id: u32,
        account: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<WorldAuthAccount>, DbError>> + Send + 'a>> {
        let row = (account == "TESTUSER").then(|| WorldAuthAccount {
            id: 42,
            security: None,
            sessionkey_hex: SESSIONKEY_HEX.to_string(),
            last_ip: "127.0.0.1".to_string(),
            mutetime: 0,
            locale: 0,
            os: "Win".to_string(),
            platform: "x86".to_string(),
            flags: 0,
            email: String::new(),
            email_verif: false,
            banned: false,
        });
        Box::pin(async move { Ok(row) })
    }

    fn update_last_ip<'a>(
        &'a self,
        account: &'a str,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>> {
        self.writes
            .lock()
            .expect("mutex")
            .push((account.to_string(), ip.to_string()));
        Box::pin(async { Ok(()) })
    }

    fn is_ip_banned<'a>(
        &'a self,
        _ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

struct FakeCharQueries;

impl CharQueries for FakeCharQueries {
    fn char_enum(&self, _account_id: u32) -> Result<Vec<CharEnumRow>, DbError> {
        Ok(Vec::new())
    }

    fn load_character(
        &self,
        _guid_low: u32,
    ) -> Result<Option<mangos_db::character_load::CharacterLoad>, DbError> {
        Ok(None)
    }

    fn global_account_data(&self, _account_id: u32) -> Result<Vec<AccountDataEntry>, DbError> {
        Ok(Vec::new())
    }

    fn tutorials(&self, _account_id: u32) -> Result<Option<[u32; 8]>, DbError> {
        Ok(None)
    }
}

struct FakePublisher;

impl RealmPublisher for FakePublisher {
    fn set_realm_population(&self, _realm_id: u32, _population: f32) -> Result<(), DbError> {
        Ok(())
    }
}

fn test_config() -> ServerConfig {
    ServerConfig::from_config(&mangos_config::Config::parse(
        b"RealmID = 1\nWorldServerPort = 0\nPlayerLimit = 0\n",
    ))
}

async fn start_server() -> u16 {
    let log = Arc::new(
        Log::open(
            std::path::Path::new(""),
            "",
            false,
            mangos_log::Level::Debug,
        )
        .expect("disabled log"),
    );
    let char_queries: Arc<dyn CharQueries> = Arc::new(FakeCharQueries);
    let (world_tx, _world, _health) = spawn_world(
        test_config(),
        Arc::clone(&log),
        mangos_world_data::strings::MangosStrings::default(),
        WorldDeps {
            enum_stores: Arc::new(EnumStores::default()),
            char_queries: Arc::clone(&char_queries),
            realm_publisher: Arc::new(FakePublisher),
        },
    );
    let backend: Arc<dyn AuthBackend> = Arc::new(FakeBackend::default());
    let deps = Arc::new(ConnectionDeps {
        config: Arc::new(test_config()),
        backend,
        char_queries,
        world: world_tx,
        log,
        server_addresses: Arc::new(HashSet::from(["127.0.0.1".to_string()])),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let mut next_id = 1u64;
        while let Ok((stream, _)) = listener.accept().await {
            let deps = Arc::clone(&deps);
            let id = SessionId(next_id);
            next_id += 1;
            tokio::spawn(run_connection(stream, id, deps));
        }
    });
    port
}

// --- raw client bits (no crypt needed pre-auth) ---------------

async fn connect(port: u16) -> TcpStream {
    TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect")
}

async fn read_challenge(stream: &mut TcpStream) -> u32 {
    let mut frame = [0u8; 8];
    timeout(Duration::from_secs(2), stream.read_exact(&mut frame))
        .await
        .expect("challenge in time")
        .expect("read challenge");
    u32::from_le_bytes(frame[4..].try_into().expect("seed"))
}

/// Outcome of poking the socket once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Poke {
    Closed,
    StillOpen,
}

async fn poke(port: u16, bytes: &[u8], wait: Duration) -> Poke {
    let mut stream = connect(port).await;
    let _ = read_challenge(&mut stream).await;
    let _ = stream.write_all(bytes).await;
    let mut buf = [0u8; 64];
    match timeout(wait, stream.read(&mut buf)).await {
        Ok(Ok(0) | Err(_)) => Poke::Closed,
        _ => Poke::StillOpen,
    }
}

fn header(size: u16, op: u32) -> [u8; 6] {
    let mut h = [0u8; 6];
    h[..2].copy_from_slice(&size.to_be_bytes());
    h[2..].copy_from_slice(&op.to_le_bytes());
    h
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

#[tokio::test]
async fn malformed_corpus_closes_or_is_ignored() {
    let port = start_server().await;
    let wait = Duration::from_millis(300);

    // size below minimum, size above maximum, opcode out of range
    assert_eq!(poke(port, &header(3, 476), wait).await, Poke::Closed);
    assert_eq!(poke(port, &header(0x2801, 476), wait).await, Poke::Closed);
    assert_eq!(poke(port, &header(4, 828), wait).await, Poke::Closed);
    // a non-auth opcode before auth
    assert_eq!(poke(port, &header(4, 55), wait).await, Poke::Closed);
    // truncated ping pre-auth: parse fails -> bad-packet policy
    // (default) keeps the connection open, like C++.
    let mut short_ping = header(7, 476).to_vec();
    short_ping.extend_from_slice(&[1, 2, 3]);
    assert_eq!(poke(port, &short_ping, wait).await, Poke::StillOpen);
}

#[tokio::test]
async fn random_bytes_never_wedge_the_server() {
    let port = start_server().await;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for case in 0..60 {
        let len = (xorshift(&mut state) % 128) as usize;
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            bytes.extend_from_slice(&xorshift(&mut state).to_le_bytes());
        }
        bytes.truncate(len);
        let _ = poke(port, &bytes, Duration::from_millis(50)).await;
        let _ = case;
    }
    // The server still completes a real login afterwards.
    valid_login(port).await;
}

#[tokio::test]
async fn every_opcode_with_empty_body_is_survived() {
    let port = start_server().await;
    // Sampled sweep on fresh authenticated connections (fresh
    // connection per opcode avoids the antiflood kick, which is
    // correct behaviour, not a crash).
    let mut opcodes: Vec<u16> = (0..828u16).step_by(7).collect();
    opcodes.extend([55, 61, 476, 493, 826, 827]);
    for op in opcodes {
        if op == opcode::CMSG_PING || op == opcode::CMSG_AUTH_SESSION {
            continue; // covered by the dedicated suites
        }
        let mut client = AuthedClient::connect(port).await;
        client.send(op, &[]).await;
        // Give the actor a tick, then prove liveness with a ping
        // OR accept a clean server-side close (both are sane).
        tokio::time::sleep(Duration::from_millis(60)).await;
        client.assert_sane().await;
    }
    valid_login(port).await;
}

/// Authenticated client with the rolling crypt (as in loopback).
struct AuthedClient {
    stream: TcpStream,
    key: Vec<u8>,
    si: usize,
    sj: u8,
    ri: usize,
    rj: u8,
}

impl AuthedClient {
    async fn connect(port: u16) -> AuthedClient {
        let mut stream = connect(port).await;
        let server_seed = read_challenge(&mut stream).await;
        let key = session_key_from_hex(SESSIONKEY_HEX).expect("key");
        let digest = expected_auth_session_digest("TESTUSER", CLIENT_SEED, server_seed, &key);
        let mut body = Vec::new();
        body.extend_from_slice(&5875u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(b"TESTUSER\0");
        body.extend_from_slice(&CLIENT_SEED.to_le_bytes());
        body.extend_from_slice(&digest);
        let mut client = AuthedClient {
            stream,
            key: key.to_bytes_le(),
            si: 0,
            sj: 0,
            ri: 0,
            rj: 0,
        };
        client
            .raw_send(opcode::CMSG_AUTH_SESSION, &body, false)
            .await;
        // AUTH_OK arrives crypted.
        let (op, _) = client.read().await.expect("auth ok");
        assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
        client
    }

    async fn raw_send(&mut self, op: u16, body: &[u8], crypt: bool) {
        let mut head = header((body.len() + 4) as u16, u32::from(op));
        if crypt {
            for byte in &mut head {
                self.si %= self.key.len();
                let k = self.key[self.si];
                self.si += 1;
                let c = (*byte ^ k).wrapping_add(self.sj);
                *byte = c;
                self.sj = c;
            }
        }
        let _ = self.stream.write_all(&head).await;
        let _ = self.stream.write_all(body).await;
    }

    async fn send(&mut self, op: u16, body: &[u8]) {
        self.raw_send(op, body, true).await;
    }

    async fn read(&mut self) -> Option<(u16, Vec<u8>)> {
        let mut head = [0u8; 4];
        timeout(Duration::from_secs(2), self.stream.read_exact(&mut head))
            .await
            .ok()?
            .ok()?;
        for byte in &mut head {
            self.ri %= self.key.len();
            let k = self.key[self.ri];
            self.ri += 1;
            let c = *byte;
            *byte = c.wrapping_sub(self.rj) ^ k;
            self.rj = c;
        }
        let size = usize::from(u16::from_be_bytes([head[0], head[1]]));
        let op = u16::from_le_bytes([head[2], head[3]]);
        let mut body = vec![0u8; size - 2];
        timeout(Duration::from_secs(2), self.stream.read_exact(&mut body))
            .await
            .ok()?
            .ok()?;
        Some((op, body))
    }

    async fn assert_sane(&mut self) {
        let mut ping = Vec::new();
        ping.extend_from_slice(&0xABCDu32.to_le_bytes());
        ping.extend_from_slice(&10u32.to_le_bytes());
        self.send(opcode::CMSG_PING, &ping).await;
        // Replies to the swept opcode (e.g. the async char-enum
        // answer) can land before the pong; drain until pong
        // or close.
        for _ in 0..4 {
            match self.read().await {
                Some((op, _)) if op == opcode::SMSG_PONG => return,
                Some(_) => {}
                None => return, // clean close is also sane
            }
        }
        panic!("no pong and no close after 4 packets");
    }
}

async fn valid_login(port: u16) {
    let mut client = AuthedClient::connect(port).await;
    client.send(opcode::CMSG_CHAR_ENUM, &[]).await;
    let (op, body) = client.read().await.expect("enum reply");
    assert_eq!(op, opcode::SMSG_CHAR_ENUM);
    assert_eq!(body, [0]);
    assert_eq!(auth_response::OK, 12);
}
