//! The world actor: single owner of all session state, driven
//! at the C++ 50 ms tick (`WorldRunnable`, architecture section
//! 3.3). Socket tasks reach it only through the bounded
//! [`WorldMessage`] channel; outbound packets flow back over a
//! per-session bounded control channel, mirroring `addSessQueue`
//! in and `WorldSocket::m_sendQueue` out.
//!
//! Phase 3 simplifications, recorded for the phases that undo
//! them: the five C++ per-class receive queues collapse into one
//! FIFO per session (cross-class reordering inside a tick is
//! unobservable while every handler is a stub), and a duplicate
//! login simply replaces the old session (`World::AddSession_`
//! without the save-on-logout machinery, which is Phase 6).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use mangos_db::DbError;
use mangos_db::character_load::CharacterLoad;
use mangos_db::characters::CharEnumRow;
use mangos_guid::ObjectGuid;
use mangos_log::{Level, Log};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcodes::opcode_meta;
use mangos_protocol::world::{WorldPacket, auth_response, opcode};
use mangos_world_data::enum_stores::EnumStores;
use mangos_world_data::strings::MangosStrings;

use crate::ServerConfig;
use crate::account_data::{AccountDataSet, PER_CHARACTER_CACHE_MASK};
use crate::auth::AcceptedAuth;
use crate::char_enum::build_char_enum;
use crate::char_queries::CharQueries;
use crate::player_login::{
    LoginGate, build_character_login_failed, build_friend_list, build_ignore_list,
    build_login_verify_world, build_sys_message, evaluate_load, motd_lines,
};
use crate::security;
use crate::session_core::{Dispatch, SessionCore};

/// C++ world tick (`WORLD_SLEEP_CONST`-class default, 50 ms).
pub const WORLD_TICK: Duration = Duration::from_millis(50);

/// Socket→world channel capacity (architecture section 3 target).
pub const WORLD_CHANNEL_CAPACITY: usize = 1024;

/// Freeze-detector replacement state: the world actor stamps
/// every tick; an external watchdog (in the binary) reads the
/// age and applies the recorded Phase 3 policy (error log
/// past 10 s, non-zero exit past 30 s). The C++ detector
/// measured the same world-loop liveness (section 3.3).
pub struct WorldHealth {
    epoch: Instant,
    last_tick_ms: AtomicU64,
    tick_count: AtomicU64,
    /// `GetActiveSessionCount` (sessions incl. queued) and its
    /// high-water mark, for the uptime row.
    active_sessions: AtomicU64,
    max_active_sessions: AtomicU64,
}

impl WorldHealth {
    fn new() -> WorldHealth {
        WorldHealth {
            epoch: Instant::now(),
            last_tick_ms: AtomicU64::new(0),
            tick_count: AtomicU64::new(0),
            active_sessions: AtomicU64::new(0),
            max_active_sessions: AtomicU64::new(0),
        }
    }

    fn stamp_tick(&self) {
        self.last_tick_ms
            .store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        self.tick_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Age of the most recent world tick.
    #[must_use]
    pub fn tick_age(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_tick_ms.load(Ordering::Relaxed)))
    }

    #[must_use]
    pub fn tick_count(&self) -> u64 {
        self.tick_count.load(Ordering::Relaxed)
    }

    fn set_active_sessions(&self, count: usize) {
        self.active_sessions.store(count as u64, Ordering::Relaxed);
        self.max_active_sessions
            .fetch_max(count as u64, Ordering::Relaxed);
    }

    #[must_use]
    pub fn active_sessions(&self) -> u32 {
        self.active_sessions.load(Ordering::Relaxed) as u32
    }

    #[must_use]
    pub fn max_active_sessions(&self) -> u32 {
        self.max_active_sessions.load(Ordering::Relaxed) as u32
    }
}

/// Stall thresholds for the watchdog (recorded decision).
pub const WATCHDOG_ERROR_AFTER: Duration = Duration::from_secs(10);
pub const WATCHDOG_EXIT_AFTER: Duration = Duration::from_secs(30);

/// Per-session outbound capacity. The C++ closes when the send
/// queue already holds more than 1024 packets at push time
/// (`WorldSocket.cpp:524`), i.e. 1025 may be queued.
pub const SESSION_SEND_CAPACITY: usize = 1025;

/// `CHEAT_ACTION_KICK` bit in the antiflood sanction mask.
const SANCTION_KICK: u32 = 0x08;

/// Monotonic session identifier (`g_sessionCounter`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

/// Initial state for a freshly authenticated session.
pub struct SessionInit {
    pub auth: AcceptedAuth,
    pub remote_ip: String,
    /// Global account-data slots loaded during auth
    /// (`LoadGlobalAccountData`).
    pub account_data: AccountDataSet,
    /// Tutorial flags loaded during auth
    /// (`LoadTutorialsData`); sent by the login packet train.
    pub tutorials: [u32; 8],
}

/// Dependencies the world actor calls out to. Grouped so the
/// spawn signature stays stable as handlers land.
pub struct WorldDeps {
    pub enum_stores: Arc<EnumStores>,
    pub char_queries: Arc<dyn CharQueries>,
    pub realm_publisher: Arc<dyn RealmPublisher>,
}

/// Login-DB writes the world performs at admission
/// (`AddSession_` population update).
pub trait RealmPublisher: Send + Sync {
    /// `UPDATE realmlist SET population = ? WHERE id = ?`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_realm_population(&self, realm_id: u32, population: f32) -> Result<(), DbError>;
}

/// Billing flags for trial-restricted accounts
/// (`WorldSession.h:96-105`): TRIAL 0x04 | RESTRICTED 0x40.
const BILLING_FLAG_TRIAL: u8 = 0x04;
const BILLING_FLAG_RESTRICTED: u8 = 0x40;

/// World→socket control flow for one session.
#[derive(Debug)]
pub enum SessionControl {
    Packet(WorldPacket),
    Disconnect,
}

/// Socket→world messages, processed in arrival order.
pub enum WorldMessage {
    AddSession {
        id: SessionId,
        init: Box<SessionInit>,
        control: mpsc::Sender<SessionControl>,
    },
    Packet {
        id: SessionId,
        packet: WorldPacket,
    },
    /// `WorldSession::SetLatency` from the socket's ping handler.
    SetLatency {
        id: SessionId,
        latency: u32,
    },
    RemoveSession {
        id: SessionId,
    },
    /// Async char-enum result (the `chrHandler` callback,
    /// delivered by account id like in C++ — the session may
    /// have been replaced since the query started).
    CharEnumReady {
        account_id: u32,
        rows: Result<Vec<CharEnumRow>, DbError>,
    },
    /// Async character-load result (the `LoginQueryHolder`
    /// callback). Keyed by session: a session that logged out
    /// mid-load must not be resurrected.
    PlayerLoginReady {
        id: SessionId,
        guid_low: u32,
        load: Result<Option<Box<CharacterLoad>>, DbError>,
    },
}

struct SessionEntry {
    account_id: u32,
    auth: AcceptedAuth,
    remote_ip: String,
    created: Instant,
    latency: u32,
    core: SessionCore,
    queue: VecDeque<WorldPacket>,
    control: mpsc::Sender<SessionControl>,
    #[allow(dead_code)] // read by the player-login packet train
    account_data: AccountDataSet,
    #[allow(dead_code)] // sent by the player-login packet train
    tutorials: [u32; 8],
    /// `m_charactersCount` from the last enum.
    characters_count: u8,
    /// `m_characterMaxLevel` from the last enum.
    character_max_level: u32,
    in_queue: bool,
    /// `PlayerLoading()` while the async load runs (also true
    /// transiently during logout in C++; Phase 3 has no logout
    /// packet yet).
    player_loading: bool,
    /// The entered character, once login completed.
    player_guid: Option<u32>,
    /// The read-only snapshot backing the empty-world session.
    #[allow(dead_code)] // consumed as handlers grow past stubs
    character_load: Option<CharacterLoad>,
}

/// `WorldSession::SendPacket` queue-full policy: `false` means
/// close the session.
fn send_packet(entry: &SessionEntry, packet: WorldPacket) -> bool {
    entry
        .control
        .try_send(SessionControl::Packet(packet))
        .is_ok()
}

struct World {
    config: ServerConfig,
    log: Arc<Log>,
    #[allow(dead_code)] // read by handlers landing in later slices
    strings: MangosStrings,
    deps: WorldDeps,
    sessions: HashMap<SessionId, SessionEntry>,
    by_account: HashMap<u32, SessionId>,
    /// Login queue, front first (`m_QueuedSessions`).
    login_queue: VecDeque<SessionId>,
    /// Last disconnect per account (queue grace period).
    last_logout: HashMap<u32, Instant>,
    /// Own sender, for async DB callbacks. Set at spawn.
    self_tx: Option<mpsc::Sender<WorldMessage>>,
    health: Arc<WorldHealth>,
}

/// Spawn the world actor task. Dropping every sender ends it.
pub fn spawn_world(
    config: ServerConfig,
    log: Arc<Log>,
    strings: MangosStrings,
    deps: WorldDeps,
) -> (mpsc::Sender<WorldMessage>, JoinHandle<()>, Arc<WorldHealth>) {
    let (tx, rx) = mpsc::channel(WORLD_CHANNEL_CAPACITY);
    let health = Arc::new(WorldHealth::new());
    let mut world = World {
        config,
        log,
        strings,
        deps,
        sessions: HashMap::new(),
        by_account: HashMap::new(),
        login_queue: VecDeque::new(),
        last_logout: HashMap::new(),
        self_tx: None,
        health: Arc::clone(&health),
    };
    world.self_tx = Some(tx.clone());
    let handle = tokio::spawn(world.run(rx));
    (tx, handle, health)
}

impl World {
    async fn run(mut self, mut rx: mpsc::Receiver<WorldMessage>) {
        let mut tick = tokio::time::interval(WORLD_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = rx.recv() => {
                    match message {
                        Some(message) => self.handle(message),
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    self.health.stamp_tick();
                    self.update_sessions();
                }
            }
        }
    }

    fn handle(&mut self, message: WorldMessage) {
        match message {
            WorldMessage::AddSession { id, init, control } => {
                let account_id = init.auth.account_id;
                if let Some(old_id) = self.by_account.get(&account_id).copied() {
                    if old_id != id {
                        self.disconnect(old_id);
                    }
                }
                self.by_account.insert(account_id, id);
                self.sessions.insert(
                    id,
                    SessionEntry {
                        account_id,
                        auth: init.auth,
                        remote_ip: init.remote_ip,
                        created: Instant::now(),
                        latency: 0,
                        core: SessionCore::new(),
                        queue: VecDeque::new(),
                        control,
                        account_data: init.account_data,
                        tutorials: init.tutorials,
                        characters_count: 10, // C++ session default
                        character_max_level: 0,
                        in_queue: false,
                        player_loading: false,
                        player_guid: None,
                        character_load: None,
                    },
                );
                self.admit(id);
                self.health.set_active_sessions(self.sessions.len());
            }
            WorldMessage::Packet { id, packet } => {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.queue.push_back(packet);
                }
            }
            WorldMessage::SetLatency { id, latency } => {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.latency = latency;
                    let _ = entry.latency; // stored like C++; consumers later
                }
            }
            WorldMessage::RemoveSession { id } => {
                self.remove(id);
                self.promote_from_queue();
                self.health.set_active_sessions(self.sessions.len());
            }
            WorldMessage::CharEnumReady { account_id, rows } => {
                self.on_char_enum_ready(account_id, rows);
            }
            WorldMessage::PlayerLoginReady { id, guid_low, load } => {
                self.on_player_login_ready(id, guid_low, load);
            }
        }
    }

    fn remove(&mut self, id: SessionId) {
        if let Some(entry) = self.sessions.remove(&id) {
            if self.by_account.get(&entry.account_id) == Some(&id) {
                self.by_account.remove(&entry.account_id);
            }
            self.login_queue.retain(|queued| *queued != id);
            self.last_logout.insert(entry.account_id, Instant::now());
        }
    }

    /// Sessions holding a world slot (not waiting in queue).
    fn admitted_count(&self) -> usize {
        self.sessions.values().filter(|e| !e.in_queue).count()
    }

    fn player_amount_limit(&self) -> u32 {
        if self.config.player_limit > 0 {
            self.config.player_limit as u32
        } else {
            0
        }
    }

    /// `HasTrialRestrictions` (`WorldSession.cpp:290`).
    fn billing_flags(&self, entry: &SessionEntry) -> u8 {
        if !entry.auth.verified_email
            && entry.auth.security == security::PLAYER
            && self.config.restrict_unverified_accounts
        {
            BILLING_FLAG_TRIAL | BILLING_FLAG_RESTRICTED
        } else {
            0
        }
    }

    /// `CanSkipQueue` (`World.cpp:3132-3144`): staff, or a
    /// recent logout inside the grace period.
    fn can_skip_queue(&self, entry: &SessionEntry) -> bool {
        if entry.auth.security > security::PLAYER {
            return true;
        }
        let grace = self.config.login_queue_grace_secs;
        if grace == 0 {
            return false;
        }
        self.last_logout.get(&entry.account_id).is_some_and(|at| {
            Instant::now().saturating_duration_since(*at) < Duration::from_secs(u64::from(grace))
        })
    }

    /// `SendAuthWaitQue` long forms from `AddSession_` /
    /// `AddQueuedSession`: code + billing fields (+ position).
    fn auth_response_long(
        &self,
        entry: &SessionEntry,
        code: u8,
        position: Option<u32>,
    ) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(code);
        writer.write_u32_le(0);
        writer.write_u8(self.billing_flags(entry));
        writer.write_u32_le(0);
        if let Some(position) = position {
            writer.write_u32_le(position);
        }
        let mut packet = WorldPacket::new(opcode::SMSG_AUTH_RESPONSE);
        packet.take_writer(writer);
        packet
    }

    /// Admission (`World::AddSession_`, `World.cpp:241-337`):
    /// queue when the player limit is full, else `AUTH_OK` plus
    /// the population publish.
    fn admit(&mut self, id: SessionId) {
        let limit = self.player_amount_limit();
        let others = self.admitted_count().saturating_sub(1);
        let must_queue = limit > 0
            && others as u32 >= limit
            && !self
                .sessions
                .get(&id)
                .is_some_and(|e| self.can_skip_queue(e));

        if must_queue {
            if let Some(entry) = self.sessions.get_mut(&id) {
                entry.in_queue = true;
                entry.core.set_in_queue(true);
            }
            self.login_queue.push_back(id);
            let position = self.login_queue.len() as u32;
            let packet = self
                .sessions
                .get(&id)
                .map(|e| self.auth_response_long(e, auth_response::WAIT_QUEUE, Some(position)));
            if let Some(packet) = packet {
                let alive = self
                    .sessions
                    .get(&id)
                    .is_some_and(|e| send_packet(e, packet));
                if !alive {
                    self.disconnect(id);
                }
            }
            return;
        }

        let packet = self
            .sessions
            .get(&id)
            .map(|e| self.auth_response_long(e, auth_response::OK, None));
        if let Some(packet) = packet {
            let alive = self
                .sessions
                .get(&id)
                .is_some_and(|e| send_packet(e, packet));
            if !alive {
                self.disconnect(id);
                return;
            }
        }
        if limit > 0 {
            #[allow(clippy::cast_precision_loss)]
            let population = self.admitted_count() as f32 / limit as f32 * 2.0;
            if let Err(e) = self
                .deps
                .realm_publisher
                .set_realm_population(self.config.realm_id, population)
            {
                self.log
                    .out(Level::Error, &format!("population publish failed: {e}"));
            }
        }
    }

    /// Queue promotion after a departure (`RemoveQueuedSession`
    /// accept path): short `AUTH_OK`, idle clock reset, then
    /// short position updates for those still waiting.
    fn promote_from_queue(&mut self) {
        let limit = self.player_amount_limit();
        let mut promoted = false;
        while limit == 0 || (self.admitted_count() as u32) < limit {
            let Some(next) = self.login_queue.pop_front() else {
                break;
            };
            let Some(entry) = self.sessions.get_mut(&next) else {
                continue;
            };
            entry.in_queue = false;
            entry.core.set_in_queue(false);
            entry.created = Instant::now();
            promoted = true;
            let packet = WorldPacket::with_body(opcode::SMSG_AUTH_RESPONSE, [auth_response::OK]);
            if !send_packet(entry, packet) {
                self.disconnect(next);
            }
        }
        if promoted {
            let waiting: Vec<SessionId> = self.login_queue.iter().copied().collect();
            for (index, queued) in waiting.iter().enumerate() {
                let mut writer = PacketWriter::new();
                writer.write_u8(auth_response::WAIT_QUEUE);
                writer.write_u32_le(index as u32 + 1);
                let mut packet = WorldPacket::new(opcode::SMSG_AUTH_RESPONSE);
                packet.take_writer(writer);
                if let Some(entry) = self.sessions.get(queued)
                    && !send_packet(entry, packet)
                {
                    self.disconnect(*queued);
                }
            }
        }
    }

    /// `HandleCharEnum` (`CharacterHandler.cpp:135-162`).
    fn on_char_enum_ready(&mut self, account_id: u32, rows: Result<Vec<CharEnumRow>, DbError>) {
        let Some(id) = self.by_account.get(&account_id).copied() else {
            return;
        };
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("char enum query failed: {e}"));
                Vec::new()
            }
        };
        let built = build_char_enum(&rows, &self.deps.enum_stores);
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        entry.characters_count = built.count;
        entry.character_max_level = entry.character_max_level.max(built.max_level);
        if !send_packet(entry, built.packet) {
            self.disconnect(id);
        }
    }

    /// Fire the async enum query (`AsyncPQuery` + `chrHandler`
    /// callback): one blocking job, result back as a message.
    fn start_char_enum(&self, account_id: u32) {
        let Some(tx) = self.self_tx.clone() else {
            return;
        };
        let queries = Arc::clone(&self.deps.char_queries);
        tokio::task::spawn_blocking(move || {
            let rows = queries.char_enum(account_id);
            let _ = tx.blocking_send(WorldMessage::CharEnumReady { account_id, rows });
        });
    }

    fn disconnect(&mut self, id: SessionId) {
        if let Some(entry) = self.sessions.get(&id) {
            let _ = entry.control.try_send(SessionControl::Disconnect);
        }
        self.remove(id);
    }

    /// `HandlePlayerLoginOpcode` (`CharacterHandler.cpp:386-420`)
    /// up to the async query dispatch.
    fn on_player_login_opcode(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw_guid) = reader.read_u64_le() else {
            self.bad_packet(id, opcode::CMSG_PLAYER_LOGIN);
            return;
        };
        let guid = ObjectGuid::from_raw(raw_guid);
        let refused = {
            let Some(entry) = self.sessions.get_mut(&id) else {
                return;
            };
            if (!self.config.world_available && entry.auth.security == security::PLAYER)
                || entry.player_loading
                || entry.player_guid.is_some()
                || !guid.is_player()
            {
                true
            } else {
                entry.player_loading = true;
                false
            }
        };
        if refused {
            let alive = self
                .sessions
                .get(&id)
                .is_some_and(|e| send_packet(e, build_character_login_failed()));
            if !alive {
                self.disconnect(id);
            }
            return;
        }
        let Some(tx) = self.self_tx.clone() else {
            return;
        };
        let queries = Arc::clone(&self.deps.char_queries);
        let guid_low = guid.counter();
        tokio::task::spawn_blocking(move || {
            let load = queries
                .load_character(guid_low)
                .map(|opt| opt.map(Box::new));
            let _ = tx.blocking_send(WorldMessage::PlayerLoginReady { id, guid_low, load });
        });
    }

    /// `HandlePlayerLogin` (`CharacterHandler.cpp:422-577`),
    /// Phase 3 cut: gates, per-character account data, then the
    /// map-independent packet train. Everything from
    /// `SendInitialPacketsBeforeAddToMap` onward (including the
    /// online-presence writes) waits for the simulated world.
    fn on_player_login_ready(
        &mut self,
        id: SessionId,
        guid_low: u32,
        load: Result<Option<Box<CharacterLoad>>, DbError>,
    ) {
        let load = match load {
            Ok(load) => load,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("character load failed: {e}"));
                None
            }
        };
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        if !entry.player_loading {
            return; // stale callback (C++ asserts this cannot happen)
        }
        entry.player_loading = false;

        match evaluate_load(load.as_deref(), entry.account_id) {
            LoginGate::Proceed => {}
            gate => {
                self.log.out(
                    Level::Error,
                    &format!(
                        "SESSION: player login refused for account {} guid {guid_low}: {gate:?}",
                        entry.account_id
                    ),
                );
                self.disconnect(id);
                return;
            }
        }
        let load = *load.expect("gate proceeded");

        entry
            .account_data
            .load(&load.account_data, PER_CHARACTER_CACHE_MASK);
        entry.player_guid = Some(guid_low);
        entry.core.set_player_state(true, false);

        let mut packets = vec![
            build_login_verify_world(&load.row),
            entry.account_data.build_md5_packet(),
            build_friend_list(&load.social),
            build_ignore_list(&load.social),
        ];
        for line in motd_lines(&self.config.motd, self.config.wow_patch) {
            packets.push(build_sys_message(&line));
        }
        entry.character_load = Some(load);

        let entry = self.sessions.get(&id).expect("present");
        for packet in packets {
            if !send_packet(entry, packet) {
                self.disconnect(id);
                return;
            }
        }
        self.log.out(
            Level::Debug,
            "SESSION: player entered the (empty) world; Phase 3 stops before map add",
        );
    }

    /// Bad-packet policy for handler parse failures
    /// (`ProcessPackets` `ByteBufferException` catch).
    fn bad_packet(&mut self, id: SessionId, opcode: u16) {
        self.log.out(
            Level::Error,
            &format!("SESSION: ByteBufferException while handling opcode {opcode}"),
        );
        if self.config.kick_on_bad_packet {
            self.disconnect(id);
        }
    }

    /// One `World::UpdateSessions` pass over sessions with work,
    /// plus the character-screen idle kick.
    fn update_sessions(&mut self) {
        let now = Instant::now();
        let idle_limit = self.config.character_screen_max_idle_secs;
        let mut kicked: Vec<SessionId> = Vec::new();
        let mut enum_requests: Vec<u32> = Vec::new();
        let mut login_requests: Vec<(SessionId, WorldPacket)> = Vec::new();

        for (id, entry) in &mut self.sessions {
            if idle_limit > 0
                && now.saturating_duration_since(entry.created)
                    >= Duration::from_secs(u64::from(idle_limit))
            {
                self.log.out(
                    Level::Debug,
                    &format!(
                        "SESSION: Kicking session [{}] from character selection",
                        entry.remote_ip
                    ),
                );
                kicked.push(*id);
                continue;
            }
            if entry.queue.is_empty() {
                continue;
            }
            entry.core.begin_update();
            while let Some(packet) = entry.queue.pop_front() {
                let opcode = packet.opcode();
                let Some(meta) = opcode_meta(opcode) else {
                    continue; // socket layer already rejects >= 828
                };
                match entry.core.process(meta, opcode) {
                    Dispatch::Execute if opcode == opcode::CMSG_CHAR_ENUM => {
                        enum_requests.push(entry.account_id);
                    }
                    Dispatch::Execute if opcode == opcode::CMSG_PLAYER_LOGIN => {
                        login_requests.push((*id, packet));
                    }
                    Dispatch::Execute => execute_stub(&self.log, entry, &packet),
                    Dispatch::DropUnexpected(reason) => self.log.out(
                        Level::Debug,
                        &format!(
                            "SESSION: received unexpected opcode {} (0x{opcode:04X}) {reason}",
                            meta.name
                        ),
                    ),
                    Dispatch::DropSilent => {}
                    Dispatch::DropNotAllowed => self.log.out(
                        Level::Error,
                        &format!(
                            "SESSION: received not allowed opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::DropUnhandled => self.log.out(
                        Level::Debug,
                        &format!(
                            "SESSION: received not handled opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::DropWrongStatus => self.log.out(
                        Level::Error,
                        &format!(
                            "SESSION: received wrong-status-req opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::FloodBreak => {
                        let flood = entry.core.flood_counters();
                        self.log.out(
                            Level::Error,
                            &format!(
                                "AntiFlood: account {} on {} sent {} packets ({} slow, {} very slow) in one update ({})",
                                entry.account_id,
                                entry.remote_ip,
                                flood.total,
                                flood.slow,
                                flood.very_slow,
                                meta.name
                            ),
                        );
                        if self.config.antiflood_sanction & SANCTION_KICK != 0 {
                            kicked.push(*id);
                        }
                        break;
                    }
                }
            }
        }
        for id in kicked {
            self.disconnect(id);
        }
        for account_id in enum_requests {
            self.start_char_enum(account_id);
        }
        for (id, packet) in login_requests {
            self.on_player_login_opcode(id, &packet);
        }
    }
}

/// Phase 3 class stub: the opcode passed the full C++
/// admission chain but no handler exists yet. Char enum is
/// real; everything else still lands here.
fn execute_stub(log: &Log, entry: &SessionEntry, packet: &WorldPacket) {
    let opcode = packet.opcode();
    let name = opcode_meta(opcode).map_or("<none>", |m| m.name);
    log.out(
        Level::Debug,
        &format!(
            "SESSION: opcode {name} (0x{opcode:04X}) from account {} reached Phase 3 stub",
            entry.account_id
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_stamp_advances() {
        let health = WorldHealth::new();
        assert_eq!(health.tick_count(), 0);
        health.stamp_tick();
        health.stamp_tick();
        assert_eq!(health.tick_count(), 2);
        assert!(health.tick_age() < Duration::from_secs(1));
    }
}
