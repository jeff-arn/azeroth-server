//! Per-connection task: the `WorldSocket` port. Owns the TCP
//! stream and both `AuthCrypt` directions (the C++ runs one IO
//! context per socket), sends the auth challenge on accept, and
//! routes packets exactly like
//! `WorldSocket::_HandleCompleteReceivedPacket`
//! (`WorldSocket.cpp:142-202`): ping and auth at the socket,
//! everything else into the world actor once a session exists.
//!
//! Deferred with evidence (character slice): `BuildAddonPacket`
//! over the auth-session addon tail (`WorldSocket.cpp:436-437`).
//! The scripted client sends no addon data, matching the C++
//! no-reply path; a real client does send it, so the owner
//! checkpoint needs that port.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use mangos_log::{Level, Log};
use mangos_protocol::opcodes::opcode_meta;
use mangos_protocol::world::{
    AuthCrypt, CLIENT_HEADER_SIZE, ClientHeader, WorldPacket, build_auth_challenge,
    build_auth_response, build_pong, opcode, parse_auth_session, parse_ping,
};

use crate::ServerConfig;
use crate::account_data::{AccountDataSet, GLOBAL_CACHE_MASK};
use crate::auth::{AuthBackend, AuthContext, AuthOutcome, authenticate};
use crate::char_queries::CharQueries;
use crate::ping::{PingAction, PingTracker};
use crate::session_core::{QueueAdmission, queue_admission};
use crate::world_actor::{
    SESSION_SEND_CAPACITY, SessionControl, SessionId, SessionInit, WorldMessage,
};

/// Shared dependencies handed to every connection task.
pub struct ConnectionDeps {
    pub config: Arc<ServerConfig>,
    pub backend: Arc<dyn AuthBackend>,
    pub char_queries: Arc<dyn CharQueries>,
    pub world: mpsc::Sender<WorldMessage>,
    pub log: Arc<Log>,
    /// `GetServerAddresses()` (`WorldSocket.cpp:208-221`). The
    /// C++ also adds this host's own hostname addresses; the
    /// loopback entry it guarantees is what the dev bench uses.
    /// Hostname expansion is deferred (see PHASE3-PROGRESS).
    pub server_addresses: Arc<HashSet<String>>,
}

fn process_epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// `WorldTimer::getMSTime`: uptime milliseconds, u32 wrapping.
fn now_ms() -> u32 {
    process_epoch().elapsed().as_millis() as u32
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flow {
    Continue,
    Close,
}

struct SessionState {
    id: SessionId,
    security: u8,
    control_rx: mpsc::Receiver<SessionControl>,
}

/// Run one accepted connection to completion.
pub async fn run_connection(mut stream: TcpStream, id: SessionId, deps: Arc<ConnectionDeps>) {
    let remote_ip = match stream.peer_addr() {
        Ok(addr) => addr.ip().to_canonical().to_string(),
        Err(_) => String::new(),
    };
    let log = &deps.log;

    let auth_seed: u32 = rand::random();
    let mut crypt = AuthCrypt::new();
    if write_packet(&mut stream, &mut crypt, &build_auth_challenge(auth_seed))
        .await
        .is_err()
    {
        return;
    }

    let mut conn = Connection {
        deps: &deps,
        remote_ip: &remote_ip,
        auth_seed,
        session: None,
        session_id: id,
        ping: PingTracker::new(),
    };
    let no_auth_timeout = deps.config.no_auth_timeout_secs;
    // One absolute deadline from accept, like the C++ one-shot
    // timer armed in WorldSocket::Start — never reset by reads.
    let no_auth_deadline_at = (no_auth_timeout > 0)
        .then(|| Instant::now() + Duration::from_secs(u64::from(no_auth_timeout)));

    let mut buf: Vec<u8> = Vec::new();
    let mut pending_header: Option<ClientHeader> = None;
    let mut chunk = [0u8; 8192];

    'outer: loop {
        // Drain every complete frame already buffered.
        match drain_frames(
            &mut conn,
            &mut stream,
            &mut crypt,
            &mut buf,
            &mut pending_header,
        )
        .await
        {
            Flow::Continue => {}
            Flow::Close => break 'outer,
        }

        let unauthenticated = conn.session.is_none();
        let no_auth_deadline = async {
            match (unauthenticated, no_auth_deadline_at) {
                (true, Some(at)) => tokio::time::sleep_until(at.into()).await,
                _ => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            read = stream.read(&mut chunk) => {
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            control = recv_control(&mut conn.session) => {
                match control {
                    Some(SessionControl::Packet(packet)) => {
                        if write_packet(&mut stream, &mut crypt, &packet).await.is_err() {
                            break;
                        }
                    }
                    Some(SessionControl::Disconnect) | None => break,
                }
            }
            () = no_auth_deadline => {
                log.out(
                    Level::Detail,
                    &format!("[{remote_ip}] Connection has reached TimeoutSecsIfNoAuth. Closing socket..."),
                );
                break;
            }
        }
    }

    if let Some(session) = conn.session.take() {
        let _ = deps
            .world
            .send(WorldMessage::RemoveSession { id: session.id })
            .await;
    }
    let _ = stream.shutdown().await;
}

/// Handle every complete frame currently in `buf`. The decoded
/// header is kept in `pending_header` while its payload is still
/// in flight, because `ClientHeader::decode` advances the crypt
/// state and must never run twice on the same bytes.
async fn drain_frames(
    conn: &mut Connection<'_>,
    stream: &mut TcpStream,
    crypt: &mut AuthCrypt,
    buf: &mut Vec<u8>,
    pending_header: &mut Option<ClientHeader>,
) -> Flow {
    loop {
        if pending_header.is_none() {
            if buf.len() < CLIENT_HEADER_SIZE {
                return Flow::Continue;
            }
            let mut wire = [0u8; CLIENT_HEADER_SIZE];
            wire.copy_from_slice(&buf[..CLIENT_HEADER_SIZE]);
            match ClientHeader::decode(crypt, &mut wire) {
                Ok(header) => {
                    buf.drain(..CLIENT_HEADER_SIZE);
                    *pending_header = Some(header);
                }
                Err(malformed) => {
                    conn.deps.log.out(
                        Level::Basic,
                        &format!(
                            "[{}] WorldSocket::DoRecvIncomingData: client sent malformed packet size = {}, cmd = {}",
                            conn.remote_ip,
                            malformed.size(),
                            malformed.opcode()
                        ),
                    );
                    return Flow::Close;
                }
            }
        }
        let Some(header) = *pending_header else {
            return Flow::Continue;
        };
        if buf.len() < header.payload_len() {
            return Flow::Continue;
        }
        let body: Vec<u8> = buf.drain(..header.payload_len()).collect();
        *pending_header = None;
        let packet = WorldPacket::received(header.opcode as u16, body, now_ms());
        match conn.handle_packet(stream, crypt, packet).await {
            Flow::Continue => {}
            Flow::Close => return Flow::Close,
        }
    }
}

async fn recv_control(session: &mut Option<SessionState>) -> Option<SessionControl> {
    match session {
        Some(state) => state.control_rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn write_packet(
    stream: &mut TcpStream,
    crypt: &mut AuthCrypt,
    packet: &WorldPacket,
) -> std::io::Result<()> {
    let frame = packet
        .encode(crypt)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "packet too large"))?;
    stream.write_all(&frame).await
}

struct Connection<'a> {
    deps: &'a ConnectionDeps,
    remote_ip: &'a str,
    auth_seed: u32,
    session: Option<SessionState>,
    session_id: SessionId,
    ping: PingTracker,
}

impl Connection<'_> {
    async fn handle_packet(
        &mut self,
        stream: &mut TcpStream,
        crypt: &mut AuthCrypt,
        packet: WorldPacket,
    ) -> Flow {
        match packet.opcode() {
            opcode::CMSG_PING => self.handle_ping(stream, crypt, &packet).await,
            opcode::CMSG_AUTH_SESSION => {
                if self.session.is_some() {
                    self.deps.log.out(
                        Level::Error,
                        "WorldSocket::ProcessIncoming: Player send CMSG_AUTH_SESSION again",
                    );
                    return Flow::Close;
                }
                self.handle_auth(stream, crypt, &packet).await
            }
            other => {
                let Some(session) = &self.session else {
                    self.deps.log.out(
                        Level::Error,
                        &format!(
                            "WorldSocket::ProcessIncoming: Client not authed opcode = {other}"
                        ),
                    );
                    return Flow::Close;
                };
                let meta = opcode_meta(other).expect("opcode < 828 checked at header");
                match queue_admission(meta, other, packet.body()) {
                    QueueAdmission::DropSilently => Flow::Continue,
                    QueueAdmission::Queue(_) => {
                        let id = session.id;
                        match self
                            .deps
                            .world
                            .send(WorldMessage::Packet { id, packet })
                            .await
                        {
                            Ok(()) => Flow::Continue,
                            Err(_) => Flow::Close,
                        }
                    }
                }
            }
        }
    }

    /// `ByteBufferException` policy for socket-handled packets
    /// (`WorldSocket.cpp:181-199`).
    fn bad_packet(&self, opcode: u16) -> Flow {
        self.deps.log.out(
            Level::Error,
            &format!(
                "WorldSocket::ProcessIncoming ByteBufferException occured while parsing an instant handled packet (opcode: {opcode}) from client {}",
                self.remote_ip
            ),
        );
        if self.deps.config.kick_on_bad_packet {
            Flow::Close
        } else {
            Flow::Continue
        }
    }

    async fn handle_ping(
        &mut self,
        stream: &mut TcpStream,
        crypt: &mut AuthCrypt,
        packet: &WorldPacket,
    ) -> Flow {
        let Ok(ping) = parse_ping(packet.body()) else {
            return self.bad_packet(opcode::CMSG_PING);
        };
        let Some(session) = &self.session else {
            self.deps.log.out(
                Level::Error,
                &format!(
                    "WorldSocket::HandlePing: peer sent CMSG_PING, but is not authenticated or got recently kicked, address = {}",
                    self.remote_ip
                ),
            );
            return Flow::Close;
        };
        let action = self.ping.observe(
            Instant::now(),
            self.deps.config.max_overspeed_pings,
            session.security,
        );
        if action == PingAction::Kick {
            self.deps.log.out(
                Level::Error,
                &format!(
                    "WorldSocket::HandlePing: Player kicked for overspeeded pings address = {}",
                    self.remote_ip
                ),
            );
            return Flow::Close;
        }
        let id = session.id;
        let _ = self
            .deps
            .world
            .send(WorldMessage::SetLatency {
                id,
                latency: ping.latency,
            })
            .await;
        match write_packet(stream, crypt, &build_pong(ping.ping)).await {
            Ok(()) => Flow::Continue,
            Err(_) => Flow::Close,
        }
    }

    async fn handle_auth(
        &mut self,
        stream: &mut TcpStream,
        crypt: &mut AuthCrypt,
        packet: &WorldPacket,
    ) -> Flow {
        let Ok(parsed) = parse_auth_session(packet.body()) else {
            return self.bad_packet(opcode::CMSG_AUTH_SESSION);
        };
        let log = &self.deps.log;
        log.out(
            Level::Debug,
            &format!(
                "WorldSocket::HandleAuthSession: client {}, serverId {}, account {}, clientseed {}",
                parsed.client_build, parsed.server_id, parsed.account, parsed.client_seed
            ),
        );

        // C++ treats a failed PQuery as "no row" (null result ->
        // AUTH_UNKNOWN_ACCOUNT); a driver error is logged and
        // folded the same way.
        let account = match self
            .deps
            .backend
            .lookup_world_account(self.deps.config.realm_id, &parsed.account)
            .await
        {
            Ok(row) => row,
            Err(e) => {
                log.out(
                    Level::Error,
                    &format!("WorldSocket::HandleAuthSession: account query failed: {e}"),
                );
                None
            }
        };
        let ip_banned = if account.is_some() {
            self.deps
                .backend
                .is_ip_banned(self.remote_ip)
                .await
                .unwrap_or(false)
        } else {
            false
        };

        let ctx = AuthContext {
            config: &self.deps.config,
            server_seed: self.auth_seed,
            remote_ip: self.remote_ip,
            remote_is_server_address: self.deps.server_addresses.contains(self.remote_ip),
            ip_banned,
        };
        let outcome = authenticate(&parsed, account.as_ref(), &ctx);
        if outcome.updates_last_ip() {
            // Fire-and-forget like PExecute; awaited here only to
            // keep ordering inside this connection.
            if let Err(e) = self
                .deps
                .backend
                .update_last_ip(&parsed.account, self.remote_ip)
                .await
            {
                log.out(
                    Level::Error,
                    &format!("WorldSocket::HandleAuthSession: last_ip update failed: {e}"),
                );
            }
        }

        match outcome {
            AuthOutcome::Reject(code) => {
                let _ = write_packet(stream, crypt, &build_auth_response(code)).await;
                Flow::Close
            }
            AuthOutcome::FailSilently | AuthOutcome::BadOsOrPlatform => Flow::Close,
            AuthOutcome::Accept(accepted) => {
                self.complete_auth(stream, crypt, *accepted, &parsed.addon_data)
                    .await
            }
        }
    }

    /// Session creation after a verified digest
    /// (`WorldSocket.cpp:417-439`): arm the crypt, load global
    /// account data + tutorials (synchronous in C++ on the IO
    /// thread; a failed query reads as empty), register with
    /// the world actor, keep the control receiver.
    async fn complete_auth(
        &mut self,
        stream: &mut TcpStream,
        crypt: &mut AuthCrypt,
        accepted: crate::auth::AcceptedAuth,
        addon_tail: &[u8],
    ) -> Flow {
        let has_addon_data = !addon_tail.is_empty();
        let log = &self.deps.log;
        log.out(
            Level::Debug,
            &format!(
                "WorldSocket::HandleAuthSession: Client '{}' authenticated successfully from {}.",
                accepted.username, self.remote_ip
            ),
        );
        crypt.set_key_biguint(&accepted.session_key);
        crypt.init();

        let account_id = accepted.account_id;
        let queries = Arc::clone(&self.deps.char_queries);
        let loaded = tokio::task::spawn_blocking(move || {
            let global = queries.global_account_data(account_id);
            let tutorials = queries.tutorials(account_id);
            (global, tutorials)
        })
        .await;
        let mut account_data = AccountDataSet::new();
        let mut tutorials = [0u32; 8];
        match loaded {
            Ok((global, tuts)) => {
                if let Ok(rows) = global {
                    account_data.load(&rows, GLOBAL_CACHE_MASK);
                }
                if let Ok(Some(flags)) = tuts {
                    tutorials = flags;
                }
            }
            Err(e) => {
                log.out(
                    Level::Error,
                    &format!("WorldSocket::HandleAuthSession: session data load failed: {e}"),
                );
            }
        }

        let (control_tx, control_rx) = mpsc::channel(SESSION_SEND_CAPACITY);
        let security_level = accepted.security;
        let init = SessionInit {
            auth: accepted,
            remote_ip: self.remote_ip.to_string(),
            account_data,
            tutorials,
        };
        if self
            .deps
            .world
            .send(WorldMessage::AddSession {
                id: self.session_id,
                init: Box::new(init),
                control: control_tx,
            })
            .await
            .is_err()
        {
            return Flow::Close;
        }
        if has_addon_data {
            // BuildAddonPacket runs here in C++ (after session
            // registration). Malformed entries throw upstream;
            // mirror with the bad-packet policy.
            match crate::addon_info::build_addon_packet(addon_tail) {
                Ok(packet) => {
                    // Write directly: this task owns the stream and
                    // the C++ sends from the IO thread too. It can
                    // race the world actor's AUTH_OK exactly like
                    // the C++ addSessQueue race.
                    if write_packet(stream, crypt, &packet).await.is_err() {
                        return Flow::Close;
                    }
                }
                Err(crate::addon_info::AddonError::MalformedEntry) => {
                    return self.bad_packet(opcode::CMSG_AUTH_SESSION);
                }
                Err(_) => {}
            }
        }
        self.session = Some(SessionState {
            id: self.session_id,
            security: security_level,
            control_rx,
        });
        Flow::Continue
    }
}

/// The address set C++ seeds with `127.0.0.1` before resolving its
/// own hostname (`WorldSocket.cpp:208-221`).
#[must_use]
pub fn default_server_addresses() -> HashSet<String> {
    HashSet::from(["127.0.0.1".to_string()])
}
