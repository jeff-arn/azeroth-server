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
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeError, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_log::{Level, Log};
use mangos_map::{Map, MapConfig, MapEvent, WorldRng, WorldStores};
use mangos_movement::movement_info::MovementInfo;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcodes::opcode_meta;
use mangos_protocol::world::{WorldPacket, auth_response, opcode};
use mangos_world_data::enum_stores::EnumStores;
use mangos_world_data::player_info::RaceClassDbc;
use mangos_world_data::spawn_support::RespawnTimes;
use mangos_world_data::strings::MangosStrings;

use crate::ServerConfig;
use crate::account_data::{AccountDataSet, PER_CHARACTER_CACHE_MASK};
use crate::auth::AcceptedAuth;
use crate::char_enum::build_char_enum;
use crate::char_queries::CharQueries;
use crate::player_login::{
    LoginGate, build_character_login_failed, build_friend_list, build_ignore_list,
    build_login_verify_world_at, build_sys_message, evaluate_load, motd_lines,
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

/// Movement ACK handler families (`MovementHandler.cpp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AckKind {
    Speed,
    FlagToggle,
    Root,
    SplineDone,
    KnockBack,
    Teleport,
}

fn ack_kind(handler_name: &str) -> Option<AckKind> {
    Some(match handler_name {
        "HandleForceSpeedChangeAckOpcodes" => AckKind::Speed,
        "HandleMovementFlagChangeToggleAck" => AckKind::FlagToggle,
        "HandleMoveRootAck" => AckKind::Root,
        "HandleMoveSplineDoneOpcode" => AckKind::SplineDone,
        "HandleMoveKnockBackAck" => AckKind::KnockBack,
        "HandleMoveTeleportAckOpcode" => AckKind::Teleport,
        _ => return None,
    })
}

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
    /// Static world stores for map simulation (Phase 4).
    pub world_stores: Arc<WorldStores>,
    /// Extracted `.map` directory (ground heights); `None`
    /// in tests that do not exercise terrain.
    pub maps_dir: Option<std::path::PathBuf>,
    /// Extracted `mmaps` directory (navmesh); `None` in
    /// tests that do not exercise pathfinding.
    pub mmaps_dir: Option<std::path::PathBuf>,
    pub race_class_dbc: Arc<RaceClassDbc>,
    /// Persisted respawn times loaded at boot.
    pub respawn_times: RespawnTimes,
    /// Phase 4 write-register statements (presence + respawn
    /// clears; owner-approved 2026-10-19).
    pub presence: Arc<dyn PresenceWriter>,
}

/// Characters-DB writes from the Phase 4 write register.
pub trait PresenceWriter: Send + Sync {
    /// `UPDATE characters SET online = ? WHERE guid = ?`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_character_online(&self, guid_low: u32, online: bool) -> Result<(), DbError>;

    /// `DELETE FROM creature_respawn WHERE guid = ? AND instance = 0`
    /// (expired row consumed at spawn; `SaveCreatureRespawnTime(0)`
    /// clears the row in this tree's saver shape for instance 0 —
    /// see the progress record).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn clear_creature_respawn(&self, guid_low: u32) -> Result<(), DbError>;

    /// `REPLACE INTO creature_respawn (guid, respawn_time, instance, map)
    /// VALUES (?, ?, 0, ?)` — `SaveCreatureRespawnTime` future-time arm
    /// (owner-approved write register, 2026-10-19).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn store_creature_respawn(
        &self,
        guid_low: u32,
        respawn_time: u64,
        map_id: u32,
    ) -> Result<(), DbError>;

    /// `DELETE FROM gameobject_respawn WHERE guid = ? AND instance = 0`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn clear_gameobject_respawn(&self, guid_low: u32) -> Result<(), DbError>;
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
    /// Simulated continent maps by map id (instance 0 only
    /// in Slice 3).
    maps: HashMap<u32, Map>,
    /// Player guid low -> map id.
    player_map: HashMap<u32, u32>,
    /// Player guid low -> session.
    player_session: HashMap<u32, SessionId>,
    /// Boot instant for the server ms clock.
    started: Instant,
    last_tick: Instant,
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
        maps: HashMap::new(),
        player_map: HashMap::new(),
        player_session: HashMap::new(),
        started: Instant::now(),
        last_tick: Instant::now(),
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
                    self.update_maps();
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
            if let Some(guid_low) = entry.player_guid {
                self.leave_map(guid_low);
            }
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
        entry.core.set_player_state(true, true);

        // Presence write (Phase 4 write register,
        // owner-approved 2026-10-19): C++ SetOnline runs
        // before the packet train.
        if let Err(e) = self.deps.presence.set_character_online(guid_low, true) {
            self.log
                .out(Level::Error, &format!("presence online write failed: {e}"));
        }

        // Materialize the aggregate and enter the map
        // (AddToMap + SendInitSelf subset). The Phase 3
        // packet order is kept: verify-world uses the
        // materialized position (homebind fallback may
        // have moved the character).
        let account_data_packet = entry.account_data.build_md5_packet();
        let social_packets = [
            build_friend_list(&load.social),
            build_ignore_list(&load.social),
        ];
        let player = match self.materialize_player(load) {
            Ok(player) => player,
            Err(e) => {
                self.log.out(
                    Level::Error,
                    &format!("SESSION: materialize failed for guid {guid_low}: {e:?}"),
                );
                let _ = self.deps.presence.set_character_online(guid_low, false);
                self.disconnect(id);
                return;
            }
        };
        let verify_position = player.unit.world.position();
        let map_id = player.unit.world.map_id;
        let player_guid = player.unit.guid();
        self.enter_map(id, player);

        let mut packets = vec![
            build_login_verify_world_at(map_id, verify_position),
            account_data_packet,
        ];
        packets.extend(social_packets);
        for line in motd_lines(&self.config.motd, self.config.wow_patch) {
            packets.push(build_sys_message(&line));
        }

        let entry = self.sessions.get(&id).expect("present");
        for packet in packets {
            if !send_packet(entry, packet) {
                self.disconnect(id);
                return;
            }
        }
        // Init-self + visibility creates queued by the map.
        if let Some(map) = self.maps.get_mut(&map_id) {
            map.build_init_self(player_guid);
        }
        self.drain_map_events();
        self.log.out(
            Level::Debug,
            &format!("SESSION: player guid {guid_low} entered map {map_id}"),
        );
    }

    /// `Player::LoadFromDB` aggregate construction with the
    /// boot-resolved DBC inputs.
    fn materialize_player(&self, load: CharacterLoad) -> Result<Player, MaterializeError> {
        let race = self
            .deps
            .race_class_dbc
            .races
            .get(&u32::from(load.row.race));
        let power_type = self
            .deps
            .race_class_dbc
            .class_power
            .get(&u32::from(load.row.class))
            .copied()
            .unwrap_or(0) as u8;
        let gender = load.row.gender & 0x01;
        let display_id = self
            .deps
            .world_stores
            .player_infos
            .create_info(u32::from(load.row.race), u32::from(load.row.class))
            .map_or(0, |c| {
                if gender == 1 {
                    c.display_id_f
                } else {
                    c.display_id_m
                }
            });
        let inputs = MaterializeInputs {
            infos: &self.deps.world_stores.player_infos,
            race,
            power_type,
            display_scale: self.deps.world_stores.scale_for_display(display_id),
            model: self.deps.world_stores.model_for_display(display_id),
            saved_map_known: true,
            saved_map_is_instance: false,
            max_money: MAX_MONEY_AMOUNT,
            now_unix: unix_now(),
        };
        Player::materialize(load, &inputs)
    }

    /// Creates (if needed) and enters the continent map.
    fn enter_map(&mut self, session: SessionId, player: Player) {
        let map_id = player.unit.world.map_id;
        let guid_low = player.unit.guid().counter();
        let now_ms = self.server_ms();
        let now_unix = unix_now();
        let maps_dir = self.deps.maps_dir.clone();
        let navmesh_dir = self.deps.mmaps_dir.clone();
        let map = self.maps.entry(map_id).or_insert_with(|| {
            let mut map = Map::new(
                map_id,
                0,
                MapConfig::default(),
                Arc::clone(&self.deps.world_stores),
                WorldRng::from_entropy(),
            );
            map.set_maps_dir(maps_dir);
            map.set_mmaps_dir(navmesh_dir);
            map
        });
        map.add_player(player, &self.deps.respawn_times, now_unix, now_ms);
        // Saved auras land after the player is in the
        // map (`Player::_LoadAuras` at login completion).
        map.apply_saved_auras(ObjectGuid::with_entry(
            mangos_guid::HighGuid::PLAYER,
            0,
            guid_low,
        ));
        self.player_map.insert(guid_low, map_id);
        self.player_session.insert(guid_low, session);
    }

    /// Removes the session's player from its map (logout
    /// and disconnect share this; character save is
    /// Phase 6).
    fn leave_map(&mut self, guid_low: u32) {
        if let Some(map_id) = self.player_map.remove(&guid_low) {
            if let Some(map) = self.maps.get_mut(&map_id) {
                let _ = map.remove_player(ObjectGuid::with_entry(
                    mangos_guid::HighGuid::PLAYER,
                    0,
                    guid_low,
                ));
            }
        }
        self.player_session.remove(&guid_low);
        if let Err(e) = self.deps.presence.set_character_online(guid_low, false) {
            self.log
                .out(Level::Error, &format!("presence offline write failed: {e}"));
        }
        self.drain_map_events();
    }

    /// Routes queued [`MapEvent`]s: packets to sessions,
    /// respawn clears to the characters DB.
    fn drain_map_events(&mut self) {
        let mut events: Vec<MapEvent> = Vec::new();
        for map in self.maps.values_mut() {
            events.extend(map.drain_events());
        }
        let mut dead: Vec<SessionId> = Vec::new();
        for event in events {
            match event {
                MapEvent::Packet { to, packet } => {
                    if let Some(id) = self.player_session.get(&to.counter()).copied()
                        && let Some(entry) = self.sessions.get(&id)
                        && !send_packet(entry, packet)
                    {
                        dead.push(id);
                    }
                }
                MapEvent::ClearCreatureRespawn { guid_low } => {
                    if let Err(e) = self.deps.presence.clear_creature_respawn(guid_low) {
                        self.log
                            .out(Level::Error, &format!("creature respawn clear failed: {e}"));
                    }
                }
                MapEvent::StoreCreatureRespawn {
                    guid_low,
                    respawn_time,
                    map_id,
                } => {
                    if let Err(e) =
                        self.deps
                            .presence
                            .store_creature_respawn(guid_low, respawn_time, map_id)
                    {
                        self.log
                            .out(Level::Error, &format!("creature respawn store failed: {e}"));
                    }
                }
                MapEvent::UnitDied { guid, killer } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "UnitDied {} killer={}",
                            guid.raw(),
                            killer.map_or(0, ObjectGuid::raw)
                        ),
                    );
                }
                // AI/script consumers land in Slices 6-7;
                // until then these are observable only.
                MapEvent::MovementInform {
                    guid,
                    motion_type,
                    point_id,
                } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "MovementInform {} type={} point={}",
                            guid.raw(),
                            motion_type,
                            point_id
                        ),
                    );
                }
                MapEvent::ChaseReached { guid, target } => {
                    self.log.out(
                        Level::Detail,
                        &format!("ChaseReached {} -> {}", guid.raw(), target.raw()),
                    );
                }
                MapEvent::JustReachedHome { guid } => {
                    self.log
                        .out(Level::Detail, &format!("JustReachedHome {}", guid.raw()));
                }
                MapEvent::WaypointScript {
                    guid,
                    script_id,
                    point_id,
                } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "WaypointScript {} script={} point={}",
                            guid.raw(),
                            script_id,
                            point_id
                        ),
                    );
                }
                MapEvent::ClearGameObjectRespawn { guid_low } => {
                    if let Err(e) = self.deps.presence.clear_gameobject_respawn(guid_low) {
                        self.log.out(
                            Level::Error,
                            &format!("gameobject respawn clear failed: {e}"),
                        );
                    }
                }
            }
        }
        for id in dead {
            self.disconnect(id);
        }
    }

    /// Map ticks with real elapsed diff (tick contract).
    fn update_maps(&mut self) {
        let now = Instant::now();
        let diff = u32::try_from(now.saturating_duration_since(self.last_tick).as_millis())
            .unwrap_or(u32::MAX);
        self.last_tick = now;
        let now_ms = self.server_ms();
        let now_unix = unix_now();
        for map in self.maps.values_mut() {
            map.tick(diff, now_ms, now_unix);
        }
        self.drain_map_events();
    }

    /// Server ms clock (mstime): monotonic from boot.
    fn server_ms(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_millis() & 0xFFFF_FFFF).unwrap_or(0)
    }

    fn player_map_guid(&self, id: SessionId) -> Option<(u32, ObjectGuid)> {
        let entry = self.sessions.get(&id)?;
        let guid_low = entry.player_guid?;
        let &map_id = self.player_map.get(&guid_low)?;
        Some((
            map_id,
            ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low),
        ))
    }

    /// `HandleCastSpellOpcode` subset: spell id +
    /// `SpellCastTargets` (u16 mask; SELF=0 or UNIT
    /// packed guid). Unknown/unsupported target forms
    /// drop silently like the unknown-spell arm.
    fn on_cast_spell(&mut self, id: SessionId, packet: &WorldPacket) {
        use mangos_map::spells::{CastOptions, CastTarget};
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 6 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let spell_id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let mask = u16::from_le_bytes([body[4], body[5]]);
        let target = if mask == 0 {
            CastTarget::SelfCast
        } else if mask & 0x0002 != 0 {
            let Ok((target_guid, _)) = ObjectGuid::read_packed(&body[6..]) else {
                self.bad_packet(id, packet.opcode());
                return;
            };
            CastTarget::Unit(target_guid)
        } else {
            return; // item/GO/location targets unsupported
        };
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.cast_spell(guid, spell_id, target, CastOptions::default());
        self.drain_map_events();
    }

    /// `HandleCancelCastOpcode` subset.
    fn on_cancel_cast(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        if packet.body().len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.cancel_cast(guid, false);
        self.drain_map_events();
    }

    /// `HandleCancelAuraOpcode` subset.
    fn on_cancel_aura(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let spell_id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.cancel_aura(guid, spell_id);
        self.drain_map_events();
    }

    /// `HandleAttackSwingOpcode`: raw target guid; the
    /// map starts melee when the target is attackable.
    fn on_attack_swing(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.attack_start(guid, target);
        self.drain_map_events();
    }

    /// `HandleAttackStopOpcode` (empty body).
    fn on_attack_stop(&mut self, id: SessionId) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.attack_stop(guid);
        self.drain_map_events();
    }

    /// `HandleSetSelectionOpcode`: raw guid into
    /// `UNIT_FIELD_TARGET`.
    fn on_set_selection(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        if let Some(player) = map.player_mut(guid) {
            player.unit.values().set_guid(
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET,
                target,
            );
        }
        self.drain_map_events();
    }

    /// `HandleEmoteOpcode`: only the hardcoded client
    /// emotes (0 = none, 3 = oneshot wave) pass, as in
    /// C++; mute/anim-cancel gates are not modeled.
    fn on_emote(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let emote = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
        if emote != 0 && emote != 3 {
            return;
        }
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.handle_emote_command(guid, emote);
        self.drain_map_events();
    }

    /// `HandleTextEmoteOpcode`: textEmote + emoteNum +
    /// target guid.
    fn on_text_emote(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 16 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let text_emote = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
        let emote_num = u32::from_le_bytes(body[4..8].try_into().expect("4 bytes"));
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[8..16].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.player_text_emote(guid, text_emote, emote_num, target);
        self.drain_map_events();
    }

    /// `HandleGameObjectUseOpcode`: one gameobject guid.
    fn on_gameobj_use(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_id, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        map.gameobject_use(guid, target);
        self.drain_map_events();
    }

    /// `HandleMovementOpcodes` relay half: parse with the
    /// player's seeded movement info (jump latch), forward
    /// to the map. Anticheat/ack opcodes are Slice 4.
    fn on_movement_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(entry) = self.sessions.get(&id) else {
            return;
        };
        let Some(guid_low) = entry.player_guid else {
            return;
        };
        let Some(&map_id) = self.player_map.get(&guid_low) else {
            return;
        };
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
        let server_ms = self.server_ms();
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        let Some(player) = map.player_mut(guid) else {
            return;
        };
        // Server is moving the unit: client movement is
        // dropped until the spline completes
        // (`HasPendingSplineDone` / not-finalized gates).
        if player.unit.pending_spline_done || !player.unit.spline.finalized() {
            return;
        }
        let mut info: MovementInfo = player.unit.world.movement_info;
        let mut reader = packet.reader();
        if info.read(&mut reader, server_ms).is_err() {
            self.bad_packet(id, packet.opcode());
            return;
        }
        map.apply_movement(guid, packet.opcode(), info, &self.deps.respawn_times);
        self.drain_map_events();
    }

    /// The movement ACK family (`MovementHandler.cpp`):
    /// force-speed, flag-toggle, root, spline-done,
    /// knockback, teleport. Common shape at 1.12.1: raw
    /// u64 mover GUID, movement counter, seeded
    /// `MovementInfo`, then the change payload. Pending
    /// changes must match exactly (counter + type +
    /// value) or the packet is dropped (C++ also feeds
    /// the anticheat wrong-ack counter — no anticheat
    /// yet, recorded). Knockback/teleport have no
    /// senders in this slice, so their pending queues
    /// are always empty and packets drop.
    #[allow(clippy::too_many_lines)] // one handler family, mirrors MovementHandler.cpp
    fn on_movement_ack(&mut self, id: SessionId, packet: &WorldPacket, kind: AckKind) {
        use mangos_entities::unit::{
            BASE_MOVE_SPEED, MOVEMENT_CHANGE_FEATHER_FALL, MOVEMENT_CHANGE_ROOT,
            MOVEMENT_CHANGE_SET_HOVER, MOVEMENT_CHANGE_WATER_WALK,
        };
        use mangos_movement::movement_info::MovementFlags;
        use mangos_protocol::opcode_table::opcode_id as op;

        let Some(entry) = self.sessions.get(&id) else {
            return;
        };
        let Some(guid_low) = entry.player_guid else {
            return;
        };
        let Some(&map_id) = self.player_map.get(&guid_low) else {
            return;
        };
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
        let server_ms = self.server_ms();
        let Some(map) = self.maps.get_mut(&map_id) else {
            return;
        };
        let opcode = packet.opcode();
        let mut reader = packet.reader();

        // Spline-done has no GUID/counter prefix.
        if kind == AckKind::SplineDone {
            let Some(player) = map.player_mut(guid) else {
                return;
            };
            let mut info: MovementInfo = player.unit.world.movement_info;
            let parsed = info
                .read(&mut reader, server_ms)
                .and_then(|()| reader.read_u32_le())
                .and_then(|spline_id| reader.read_f32_le().map(|_| spline_id));
            let Ok(spline_id) = parsed else {
                self.bad_packet(id, opcode);
                return;
            };
            if player.unit.spline.id() != spline_id || !player.unit.pending_spline_done {
                return;
            }
            player.unit.pending_spline_done = false;
            map.relocate_from_client(guid, info, &self.deps.respawn_times);
            self.drain_map_events();
            return;
        }

        let Some(player) = map.player_mut(guid) else {
            return;
        };
        let mut info: MovementInfo = player.unit.world.movement_info;
        let parsed: Result<(ObjectGuid, u32, f32, bool), mangos_protocol::buffer::ReadError> =
            (|| {
                let raw = reader.read_u64_le()?;
                let counter = reader.read_u32_le()?;
                let mover = ObjectGuid::from_wire_raw(raw);
                if kind == AckKind::Teleport {
                    // guid, counter, time — no MovementInfo.
                    let _time = reader.read_u32_le()?;
                    return Ok((mover, counter, 0.0f32, false));
                }
                info.read(&mut reader, server_ms)?;
                let (value, apply) = match kind {
                    AckKind::Speed => (reader.read_f32_le()?, false),
                    AckKind::FlagToggle => (0.0, reader.read_u32_le()? != 0),
                    _ => (0.0, false),
                };
                Ok((mover, counter, value, apply))
            })();
        let Ok((mover, counter, value, apply)) = parsed else {
            self.bad_packet(id, opcode);
            return;
        };
        // GetMoverFromGuid: only the session's own player
        // is a mover in this slice (no possession).
        if mover != guid {
            return;
        }

        let wrong_ack = |log: &Log, what: &str| {
            log.out(
                Level::Error,
                &format!("{what}: unexpected ack opcode {opcode} counter {counter}"),
            );
        };
        match kind {
            AckKind::Speed => {
                let move_type = match opcode {
                    op::CMSG_FORCE_WALK_SPEED_CHANGE_ACK => 0,
                    op::CMSG_FORCE_RUN_SPEED_CHANGE_ACK => 1,
                    op::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK => 2,
                    op::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK => 3,
                    op::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK => 4,
                    op::CMSG_FORCE_TURN_RATE_CHANGE_ACK => 5,
                    _ => return,
                };
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_speed_change(value, counter, move_type)
                {
                    wrong_ack(&self.log, "HandleForceSpeedChangeAck");
                    return;
                }
                // Speed applies before relocation.
                map.set_speed_rate_real(guid, move_type, value / BASE_MOVE_SPEED[move_type]);
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.send_speed_change_to_observers(guid, move_type, value);
            }
            AckKind::FlagToggle => {
                let (change_type, flag) = match opcode {
                    op::CMSG_MOVE_WATER_WALK_ACK => {
                        (MOVEMENT_CHANGE_WATER_WALK, MovementFlags::WATER_WALKING)
                    }
                    op::CMSG_MOVE_HOVER_ACK => (MOVEMENT_CHANGE_SET_HOVER, MovementFlags::HOVER),
                    op::CMSG_MOVE_FEATHER_FALL_ACK => {
                        (MOVEMENT_CHANGE_FEATHER_FALL, MovementFlags::SAFE_FALL)
                    }
                    _ => return,
                };
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_flag_change(change_type, counter, apply)
                {
                    wrong_ack(&self.log, "HandleMovementFlagChangeToggleAck");
                    return;
                }
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.set_movement_flag_real(guid, flag, apply);
                map.send_flag_change_to_observers(guid, flag, apply);
            }
            AckKind::Root => {
                let apply = opcode == op::CMSG_FORCE_MOVE_ROOT_ACK;
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_flag_change(MOVEMENT_CHANGE_ROOT, counter, apply)
                {
                    wrong_ack(&self.log, "HandleMoveRootAck");
                    return;
                }
                // 1.14-client root-on-landing workaround
                // (kick when the ack lacks ROOT and the
                // unit is not falling) is anticheat-side;
                // recorded as not reproduced.
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.set_movement_flag_real(guid, MovementFlags::ROOT, apply);
                map.send_flag_change_to_observers(guid, MovementFlags::ROOT, apply);
            }
            AckKind::KnockBack | AckKind::Teleport => {
                // No senders yet: nothing can be pending.
                wrong_ack(&self.log, "HandleMoveAck(no sender)");
                return;
            }
            AckKind::SplineDone => unreachable!("handled above"),
        }
        self.drain_map_events();
    }

    /// `HandleLogoutRequestOpcode` immediate path (combat
    /// and duels do not exist yet): logout complete +
    /// leave map + back to character screen state.
    fn on_logout_request(&mut self, id: SessionId) {
        let guid_low = self.sessions.get(&id).and_then(|e| e.player_guid);
        let Some(guid_low) = guid_low else {
            return;
        };
        self.leave_map(guid_low);
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.player_guid = None;
            entry.character_load = None;
            entry.core.set_player_state(false, false);
            entry.core.set_recently_logged_out(true);
            let packet = WorldPacket::new(opcode::SMSG_LOGOUT_COMPLETE);
            if !send_packet(entry, packet) {
                self.disconnect(id);
            }
        }
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
    #[allow(clippy::too_many_lines)] // one dispatch pass, mirrors ProcessPackets
    fn update_sessions(&mut self) {
        let now = Instant::now();
        let idle_limit = self.config.character_screen_max_idle_secs;
        let mut kicked: Vec<SessionId> = Vec::new();
        let mut enum_requests: Vec<u32> = Vec::new();
        let mut login_requests: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut movement_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut ack_packets: Vec<(SessionId, WorldPacket, AckKind)> = Vec::new();
        let mut cast_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut cancel_cast_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut cancel_aura_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut attack_swing_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut attack_stop_packets: Vec<SessionId> = Vec::new();
        let mut set_selection_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut emote_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut text_emote_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut gameobj_use_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut logout_requests: Vec<SessionId> = Vec::new();

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
                    Dispatch::Execute if opcode == opcode::CMSG_LOGOUT_REQUEST => {
                        logout_requests.push(*id);
                    }
                    Dispatch::Execute if meta.handler_name == "HandleMovementOpcodes" => {
                        movement_packets.push((*id, packet));
                    }
                    Dispatch::Execute if ack_kind(meta.handler_name).is_some() => {
                        let kind = ack_kind(meta.handler_name).expect("checked");
                        ack_packets.push((*id, packet, kind));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCastSpellOpcode" => {
                        cast_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCancelCastOpcode" => {
                        cancel_cast_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCancelAuraOpcode" => {
                        cancel_aura_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleAttackSwingOpcode" => {
                        attack_swing_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleAttackStopOpcode" => {
                        attack_stop_packets.push(*id);
                    }
                    Dispatch::Execute if meta.handler_name == "HandleSetSelectionOpcode" => {
                        set_selection_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleEmoteOpcode" => {
                        emote_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleTextEmoteOpcode" => {
                        text_emote_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleGameObjectUseOpcode" => {
                        gameobj_use_packets.push((*id, packet));
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
        for (id, packet) in movement_packets {
            self.on_movement_packet(id, &packet);
        }
        for (id, packet, kind) in ack_packets {
            self.on_movement_ack(id, &packet, kind);
        }
        for (id, packet) in cast_packets {
            self.on_cast_spell(id, &packet);
        }
        for (id, packet) in cancel_cast_packets {
            self.on_cancel_cast(id, &packet);
        }
        for (id, packet) in cancel_aura_packets {
            self.on_cancel_aura(id, &packet);
        }
        for (id, packet) in attack_swing_packets {
            self.on_attack_swing(id, &packet);
        }
        for id in attack_stop_packets {
            self.on_attack_stop(id);
        }
        for (id, packet) in set_selection_packets {
            self.on_set_selection(id, &packet);
        }
        for (id, packet) in emote_packets {
            self.on_emote(id, &packet);
        }
        for (id, packet) in text_emote_packets {
            self.on_text_emote(id, &packet);
        }
        for (id, packet) in gameobj_use_packets {
            self.on_gameobj_use(id, &packet);
        }
        for id in logout_requests {
            self.on_logout_request(id);
        }
    }
}

/// Wall-clock unix seconds (`time(nullptr)`).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
