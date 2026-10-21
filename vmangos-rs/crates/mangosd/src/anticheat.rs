//! Movement anticheat (`MovementAnticheat`) subset:
//! flag tests, position tests, cheat counting with
//! the 4 s finalize tick, and reject semantics, ported
//! from `Anticheat/MovementAnticheat/MovementAnticheat.cpp`
//! with config defaults from `World.cpp` (movement
//! anticheat block). Not ported (recorded): botting
//! heuristics, forbidden-area (battlegrounds are out
//! of goal scope), explore/unreachable hooks, packet
//! log/sniff dump, ban/mute action writes (kick and
//! log are wired; ban bits degrade to kick in
//! `world_actor`), wall-climb mid-slope vmap arm
//! (no static-vs-vmap height split in the Rust
//! terrain layer), spline/transport extrapolation
//! arms (no player transport runtime yet).

use mangos_config::Config;
use mangos_movement::movement_info::{JumpInfo, MovementFlags, MovementInfo};

/// `enum CheatType` bit positions.
pub mod cheat {
    pub const TIME_BACK: u32 = 1 << 0;
    pub const NULL_CLIENT_TIME: u32 = 1 << 1;
    pub const SKIPPED_HEARTBEATS: u32 = 1 << 2;
    pub const TIME_DESYNC: u32 = 1 << 3;
    pub const NUM_DESYNC: u32 = 1 << 4;
    pub const OVERSPEED_DIST: u32 = 1 << 5;
    pub const OVERSPEED_JUMP: u32 = 1 << 6;
    pub const JUMP_SPEED_CHANGE: u32 = 1 << 7;
    pub const MULTI_JUMP: u32 = 1 << 8;
    pub const WALL_CLIMB: u32 = 1 << 9;
    pub const FLY_HACK_SWIM: u32 = 1 << 11;
    pub const NO_FALL_TIME: u32 = 1 << 12;
    pub const BAD_FALL_RESET: u32 = 1 << 13;
    pub const BAD_FALL_STOP: u32 = 1 << 14;
    pub const BAD_MOVE_START: u32 = 1 << 15;
    pub const TELEPORT: u32 = 1 << 16;
    pub const TELEPORT_TRANSPORT: u32 = 1 << 17;
    pub const FAKE_TRANSPORT: u32 = 1 << 18;
    pub const WATER_WALK: u32 = 1 << 19;
    pub const SLOW_FALL: u32 = 1 << 20;
    pub const HOVER: u32 = 1 << 21;
    pub const FIXED_Z: u32 = 1 << 22;
    pub const ROOT_MOVE: u32 = 1 << 23;
    pub const SELF_ROOT: u32 = 1 << 24;
    pub const WRONG_ACK_DATA: u32 = 1 << 25;
}

/// `CHEATS_COUNT` (occurrence array size).
const CHEATS_COUNT: usize = 31;
/// `CHEATS_UPDATE_INTERVAL`.
const UPDATE_INTERVAL_MS: u32 = 4000;
/// `JUMP_FLAG_THRESHOLD`.
const JUMP_FLAG_THRESHOLD: u32 = 3;
/// `MIN_FALLING_TIME`.
const MIN_FALLING_TIME: u32 = 1000;
/// `ALLOWED_TRANSPORT_DISTANCE`.
const ALLOWED_TRANSPORT_DISTANCE: f32 = 100.0;

/// `CHEAT_ACTION_*` bits.
pub const ACTION_LOG: u32 = 0x01;
pub const ACTION_REPORT_GMS: u32 = 0x02;
pub const ACTION_GLOBAL_ANNOUNCE: u32 = 0x04;
pub const ACTION_KICK: u32 = 0x08;
pub const ACTION_BAN_ACCOUNT: u32 = 0x10;
pub const ACTION_BAN_IP_ACCOUNT: u32 = 0x20;

const PENALTY_KICK: u32 = ACTION_LOG | ACTION_REPORT_GMS | ACTION_KICK;
const PENALTY_LOG: u32 = ACTION_LOG | ACTION_REPORT_GMS;

/// One cheat's config row. `threshold_tick`/`total`
/// of 0 = unused (fork only wires the listed side).
#[derive(Clone, Copy, Debug)]
struct CheatCfg {
    enable: bool,
    reject: bool,
    threshold_tick: u32,
    threshold_total: u32,
    penalty: u32,
}

/// `Anticheat.*` config family (fork defaults).
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // fork Anticheat.* key family
pub struct AnticheatConfig {
    pub enabled: bool,
    pub players_only: bool,
    pub notify_cheaters: bool,
    pub ban_duration: u32,
    pub log_data: bool,
    pub max_allowed_desync: i32,
    pub teleport_distance: f32,
    pub wall_slope: f32,
    pub wall_slope_high: f32,
    speed_hack_enable: bool,
    cheats: [CheatCfg; CHEATS_COUNT],
}

fn to_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

#[allow(clippy::cast_possible_wrap)]
const fn d_as_i32(value: u32) -> i32 {
    value as i32
}

impl AnticheatConfig {
    #[must_use]
    #[allow(clippy::too_many_lines)] // config table transcription
    pub fn from_config(cfg: &Config) -> Self {
        let mut cheats = [CheatCfg {
            enable: true,
            reject: false,
            threshold_tick: 0,
            threshold_total: 0,
            penalty: PENALTY_KICK,
        }; CHEATS_COUNT];
        let mut row = |idx: usize,
                       key: &str,
                       def_enable: bool,
                       reject_key: Option<&str>,
                       tick: Option<(&str, u32)>,
                       total: Option<(&str, u32)>,
                       def_penalty: u32| {
            cheats[idx] = CheatCfg {
                enable: cfg.bool_default(&format!("Anticheat.{key}.Enable"), def_enable),
                reject: reject_key
                    .is_some_and(|k| cfg.bool_default(&format!("Anticheat.{k}.Reject"), true)),
                threshold_tick: tick.map_or(0, |(k, d)| {
                    to_u32(cfg.int_default(&format!("Anticheat.{k}"), d_as_i32(d)))
                }),
                threshold_total: total.map_or(0, |(k, d)| {
                    to_u32(cfg.int_default(&format!("Anticheat.{k}"), d_as_i32(d)))
                }),
                penalty: to_u32(
                    cfg.int_default(&format!("Anticheat.{key}.Penalty"), d_as_i32(def_penalty)),
                ),
            };
        };
        // Index = CheatType enum position.
        row(
            0,
            "ReverseTime",
            true,
            None,
            Some(("ReverseTime.Threshold", 1)),
            None,
            PENALTY_KICK,
        );
        row(
            1,
            "NullTime",
            true,
            None,
            Some(("NullTime.Threshold", 2)),
            None,
            PENALTY_KICK,
        );
        row(
            2,
            "SkippedHeartbeats",
            true,
            None,
            Some(("SkippedHeartbeats.Threshold.Tick", 2)),
            Some(("SkippedHeartbeats.Threshold.Total", 10)),
            PENALTY_KICK,
        );
        row(
            3,
            "TimeDesync",
            true,
            None,
            Some(("TimeDesync.Threshold", 5000)),
            None,
            PENALTY_KICK,
        );
        row(
            4,
            "NumDesyncs",
            true,
            None,
            None,
            Some(("NumDesyncs.Threshold", 5)),
            PENALTY_KICK,
        );
        row(
            5,
            "OverpspeedDistance",
            true,
            None,
            Some(("OverpspeedDistance.Threshold", 30)),
            None,
            PENALTY_KICK,
        );
        row(
            6,
            "OverspeedJump",
            true,
            Some("OverspeedJump"),
            None,
            Some(("OverspeedJump.Threshold", 3)),
            PENALTY_KICK,
        );
        row(
            7,
            "JumpSpeedChange",
            true,
            Some("JumpSpeedChange"),
            None,
            Some(("JumpSpeedChange.Threshold", 3)),
            PENALTY_KICK,
        );
        row(
            8,
            "MultiJump",
            true,
            Some("MultiJump"),
            Some(("MultiJump.Threshold.Tick", 2)),
            Some(("MultiJump.Threshold.Total", 10)),
            PENALTY_KICK,
        );
        row(
            9,
            "WallClimb",
            true,
            Some("WallClimb"),
            Some(("WallClimb.Threshold.Tick", 3)),
            Some(("WallClimb.Threshold.Total", 30)),
            PENALTY_KICK,
        );
        // 10 = PVE_FLYHACK (unreachable hook, unported).
        row(
            11,
            "FlyHack",
            true,
            Some("FlyHack"),
            Some(("FlyHack.Threshold", 3)),
            None,
            PENALTY_KICK,
        );
        row(
            12,
            "NoFallTime",
            true,
            None,
            None,
            Some(("NoFallTime.Threshold", 1)),
            PENALTY_KICK,
        );
        row(
            13,
            "BadFallReset",
            true,
            None,
            None,
            Some(("BadFallReset.Threshold", 1)),
            PENALTY_KICK,
        );
        row(
            14,
            "BadFallStop",
            true,
            None,
            None,
            Some(("BadFallStop.Threshold", 1)),
            PENALTY_KICK,
        );
        row(
            15,
            "BadMoveStart",
            true,
            None,
            None,
            Some(("BadMoveStart.Threshold", 1)),
            PENALTY_KICK,
        );
        // Teleport penalty default carries BAN_ACCOUNT in the fork.
        row(
            16,
            "Teleport",
            true,
            Some("Teleport"),
            None,
            Some(("Teleport.Threshold", 3)),
            ACTION_LOG | ACTION_REPORT_GMS | ACTION_BAN_ACCOUNT,
        );
        row(
            17,
            "TeleportToTransport",
            true,
            Some("TeleportToTransport"),
            None,
            Some(("TeleportToTransport.Threshold", 2)),
            PENALTY_KICK,
        );
        row(
            18,
            "FakeTransport",
            true,
            Some("FakeTransport"),
            Some(("FakeTransport.Threshold", 1)),
            None,
            PENALTY_LOG,
        );
        row(
            19,
            "WaterWalk",
            true,
            Some("WaterWalk"),
            Some(("WaterWalk.Threshold", 5)),
            None,
            PENALTY_KICK,
        );
        row(
            20,
            "SlowFall",
            true,
            Some("SlowFall"),
            Some(("SlowFall.Threshold", 5)),
            None,
            PENALTY_KICK,
        );
        row(
            21,
            "Hover",
            true,
            Some("Hover"),
            Some(("Hover.Threshold", 5)),
            None,
            PENALTY_KICK,
        );
        row(
            22,
            "FixedZ",
            true,
            Some("FixedZ"),
            Some(("FixedZ.Threshold", 5)),
            None,
            PENALTY_KICK,
        );
        row(
            23,
            "RootMove",
            true,
            Some("RootMove"),
            Some(("RootMove.Threshold.Tick", 5)),
            Some(("RootMove.Threshold.Total", 30)),
            PENALTY_KICK,
        );
        row(
            24,
            "SelfRoot",
            true,
            Some("SelfRoot"),
            Some(("SelfRoot.Threshold", 1)),
            None,
            PENALTY_LOG,
        );
        row(
            25,
            "WrongAckData",
            true,
            None,
            None,
            Some(("WrongAckData.Threshold", 3)),
            PENALTY_KICK,
        );
        let angle = cfg.float_default("Anticheat.WallClimb.Angle", 1.0);
        Self {
            enabled: cfg.bool_default("Anticheat.Enable", false),
            players_only: cfg.bool_default("Anticheat.PlayersOnly", true),
            notify_cheaters: cfg.bool_default("Anticheat.NotifyCheaters", false),
            ban_duration: to_u32(cfg.int_default("Anticheat.BanDuration", 86400)),
            log_data: cfg.bool_default("Anticheat.LogData", false),
            max_allowed_desync: cfg.int_default("Anticheat.MaxAllowedDesync", 0),
            teleport_distance: cfg.float_default("Anticheat.Teleport.Distance", 40.0),
            wall_slope: angle.tan(),
            wall_slope_high: (angle + 0.2).tan(),
            speed_hack_enable: cfg.bool_default("Anticheat.SpeedHack.Enable", true),
            cheats,
        }
    }

    fn cheat_cfg(&self, bit: u32) -> &CheatCfg {
        &self.cheats[bit.trailing_zeros() as usize]
    }

    /// `ShouldRejectMovement`: any flagged cheat whose
    /// per-cheat Reject config is on.
    fn should_reject(&self, cheat_flags: u32) -> bool {
        [
            cheat::OVERSPEED_JUMP,
            cheat::JUMP_SPEED_CHANGE,
            cheat::MULTI_JUMP,
            cheat::WALL_CLIMB,
            cheat::FLY_HACK_SWIM,
            cheat::TELEPORT,
            cheat::TELEPORT_TRANSPORT,
            cheat::FAKE_TRANSPORT,
            cheat::WATER_WALK,
            cheat::SLOW_FALL,
            cheat::HOVER,
            cheat::FIXED_Z,
            cheat::ROOT_MOVE,
            cheat::SELF_ROOT,
        ]
        .iter()
        .any(|&bit| cheat_flags & bit != 0 && self.cheat_cfg(bit).reject)
    }
}

/// Unit facts the tests read, gathered by the caller
/// from the map player (fork reads `me` directly).
#[allow(clippy::struct_excessive_bools)] // fork Unit fact bundle
pub struct MoveFacts {
    /// Session security (`SEC_PLAYER` = 0).
    pub security: u32,
    /// Server-side unit position (teleport distance).
    pub server_pos: (f32, f32, f32),
    /// `GetSpeed` per move type + `GetSpeedRate`.
    pub speeds: [f32; 6],
    pub speed_rates: [f32; 6],
    pub pending_root: bool,
    pub pending_water_walk: bool,
    pub pending_feather_fall: bool,
    pub pending_hover: bool,
    /// Root/stun unit states (holder-derived).
    pub rooted_or_stunned: bool,
    pub has_water_walk_aura: bool,
    pub has_ghost_aura: bool,
    pub has_feather_fall_aura: bool,
    pub has_hover_aura: bool,
    /// Server-side transport presence (`m_transport`).
    pub on_transport_server: bool,
    pub being_teleported: bool,
    pub spline_finalized: bool,
    /// Undercity/Deeprun/Thousand Needles lift areas.
    pub in_transport_area: bool,
    /// Claimed transport exists, is a transport, and
    /// is within 70 yd (fake-transport check).
    pub claimed_transport_ok: bool,
    /// Extrapolated position from the last info over
    /// the client-time diff (speed-hack check);
    /// `None` mirrors fork `ExtrapolateMovement`
    /// returning false.
    pub extrapolated: Option<(f32, f32, f32)>,
}

/// Per-session anticheat state (`MovementAnticheat`
/// member subset). `last` mirrors `me->m_movementInfo`
/// as the tests see it: the caller passes the unit's
/// stored info (updated only by accepted packets).
#[derive(Default)]
pub struct MovementAnticheat {
    /// `m_jumpInitialSpeed` latch (fork sets it on
    /// jump/land in the movement handler).
    pub jump_initial_speed: f32,
    knock_back: bool,
    jump_count: u32,
    jump_flag_count: u32,
    jump_flag_time: u32,
    client_desync: i32,
    overspeed_distance: f32,
    update_timer_ms: u32,
    tick_counts: [u32; CHEATS_COUNT],
    total_counts: [u32; CHEATS_COUNT],
}

/// Test outcome: fork returns a reject-until
/// timestamp (0 = accept).
pub enum MoveVerdict {
    Accept,
    Reject,
}

fn was_sent_by_session(info: &MovementInfo, session_guid: u32) -> bool {
    info.client_time != 0 && info.source_session_guid == session_guid
}

/// `GetMoveTypeForMovementInfo`.
fn move_type_for(info: &MovementInfo) -> usize {
    // MOVE_WALK 0, RUN 1, RUN_BACK 2, SWIM 3, SWIM_BACK 4, TURN_RATE 5.
    if info.flags.has(MovementFlags::SWIMMING) {
        if info.flags.has(MovementFlags::BACKWARD) {
            4
        } else {
            3
        }
    } else if info.flags.has(MovementFlags::WALK_MODE) {
        0
    } else if info.flags.has(MovementFlags::MASK_MOVING) {
        if info.flags.has(MovementFlags::BACKWARD) {
            2
        } else {
            1
        }
    } else {
        1
    }
}

fn dist2d(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl MovementAnticheat {
    fn store(&mut self, cheat_flags: u32) {
        for i in 0..CHEATS_COUNT {
            if cheat_flags & (1 << i) != 0 {
                self.tick_counts[i] += 1;
                self.total_counts[i] += 1;
            }
        }
    }

    /// `OnWrongAckData` counting hook.
    pub fn on_wrong_ack_data(&mut self, cfg: &AnticheatConfig, security: u32) {
        if !cfg.enabled || (cfg.players_only && security != 0) {
            return;
        }
        if cfg.cheat_cfg(cheat::WRONG_ACK_DATA).enable {
            self.store(cheat::WRONG_ACK_DATA);
        }
    }

    /// Reset jump counters (reject path).
    fn reset_jump_counters(&mut self) {
        self.jump_count = 0;
        self.jump_flag_count = 0;
        self.jump_flag_time = 0;
    }

    #[must_use]
    pub fn jump_counters_active(&self) -> bool {
        self.jump_count > 0
    }

    /// `Update` 4 s tick: returns (action, reason)
    /// when the finalize fires.
    pub fn update(
        &mut self,
        cfg: &AnticheatConfig,
        diff_ms: u32,
    ) -> Option<(u32, String, i32, f32)> {
        if !cfg.enabled {
            return None;
        }
        if self.update_timer_ms >= diff_ms {
            self.update_timer_ms -= diff_ms;
            return None;
        }
        Some(self.finalize(cfg))
    }

    /// `Finalize` + `ComputeCheatAction`.
    pub fn finalize(&mut self, cfg: &AnticheatConfig) -> (u32, String, i32, f32) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if self.overspeed_distance > 0.0 {
            self.tick_counts[cheat::OVERSPEED_DIST.trailing_zeros() as usize] =
                self.overspeed_distance as u32;
        }
        if self.client_desync > 0 {
            #[allow(clippy::cast_sign_loss)]
            {
                self.tick_counts[cheat::TIME_DESYNC.trailing_zeros() as usize] =
                    self.client_desync as u32;
            }
        }
        self.update_timer_ms = UPDATE_INTERVAL_MS;
        let mut action = 0u32;
        let mut reason = String::new();
        for i in 0..CHEATS_COUNT {
            let row = cfg.cheats[i];
            if !row.enable {
                continue;
            }
            for (total, threshold, count) in [
                (false, row.threshold_tick, self.tick_counts[i]),
                (true, row.threshold_total, self.total_counts[i]),
            ] {
                if threshold != 0 && count >= threshold {
                    if !reason.is_empty() {
                        reason.push('/');
                    }
                    {
                        use std::fmt::Write as _;
                        let _ = write!(
                            reason,
                            "{}({}:{})",
                            cheat_name(i),
                            if total { "Total" } else { "Tick" },
                            count
                        );
                    }
                    action |= row.penalty;
                    if total {
                        self.total_counts[i] = 0;
                    }
                }
            }
        }
        let desync = self.client_desync;
        let dist = self.overspeed_distance;
        self.tick_counts = [0; CHEATS_COUNT];
        self.client_desync = 0;
        self.overspeed_distance = 0.0;
        (action, reason, desync, dist)
    }
}

/// `GetMovementCheatName` (subset used in reasons).
fn cheat_name(idx: usize) -> &'static str {
    [
        "ReverseTime",
        "NullTime",
        "SkippedHeartbeats",
        "TimeDesync",
        "NumDesyncs",
        "OverspeedDistance",
        "OverspeedJump",
        "JumpSpeedChange",
        "MultiJump",
        "WallClimb",
        "Unreachable",
        "FlyHack",
        "NoFallTime",
        "BadFallReset",
        "BadFallStop",
        "BadMoveStart",
        "Teleport",
        "TeleportToTransport",
        "FakeTransport",
        "WaterWalk",
        "SlowFall",
        "Hover",
        "FixedZ",
        "RootMove",
        "SelfRoot",
        "WrongAckData",
        "PendingAckDelay",
        "ExploreArea",
        "ExploreHighLevelArea",
        "ForbiddenArea",
        "Botting",
    ][idx]
}

/// Jump/fall extrapolation helper inputs the caller
/// resolves against the map (ground height).
#[must_use]
pub fn compute_fall_elevation(t: f32, safe_fall: bool, start_velocity: f32) -> f32 {
    // `Movement::computeFallElevation` (util.cpp):
    // gravity 19.29110527038574, terminalVelocity
    // 60.148003, terminalSavefallVelocity 7.
    const GRAVITY: f32 = 19.291_105;
    const TERMINAL_VELOCITY: f32 = 60.148_003;
    let term_vel = if safe_fall { 7.0 } else { TERMINAL_VELOCITY };
    let start_velocity = start_velocity.min(term_vel);
    #[allow(clippy::cast_possible_truncation)]
    let terminal_time =
        (TERMINAL_VELOCITY as f64 / 19.291_105_270_385_74) as f32 - start_velocity / GRAVITY;
    if t > terminal_time {
        TERMINAL_VELOCITY * (t - terminal_time)
            + start_velocity * terminal_time
            + GRAVITY * terminal_time * terminal_time * 0.5
    } else {
        start_velocity * t + GRAVITY * t * t * 0.5
    }
}

impl MovementAnticheat {
    /// `HandleFlagTests`. Mutates `info` flags on the
    /// strip path; `Reject` asks the caller to drop
    /// the packet, force a heartbeat, and open the
    /// reject window.
    pub fn flag_tests(
        &mut self,
        cfg: &AnticheatConfig,
        facts: &MoveFacts,
        last: &MovementInfo,
        info: &mut MovementInfo,
        opcode: u16,
    ) -> (MoveVerdict, u32) {
        use mangos_protocol::opcode_table::opcode_id as op;
        if !cfg.enabled || (cfg.players_only && facts.security != 0) {
            return (MoveVerdict::Accept, 0);
        }
        let flags = info.flags;
        let mut remove: u32 = MovementFlags::NONE;
        let mut cheats = 0u32;
        // Client rooted without server root.
        if flags.has(MovementFlags::ROOT)
            && !last.flags.has(MovementFlags::ROOT)
            && !facts.pending_root
            && !facts.rooted_or_stunned
            && opcode != op::CMSG_FORCE_MOVE_ROOT_ACK
        {
            cheats |= cheat::SELF_ROOT;
            remove |= MovementFlags::ROOT;
        }
        // Moving while rooted.
        if flags.has(MovementFlags::MASK_MOVING)
            && (flags.has(MovementFlags::ROOT) || last.flags.has(MovementFlags::ROOT))
            && !facts.pending_root
            && opcode != op::CMSG_FORCE_MOVE_UNROOT_ACK
        {
            cheats |= cheat::ROOT_MOVE;
            remove |= MovementFlags::MASK_MOVING;
            info.flags.add(MovementFlags::ROOT);
        }
        // Creature-only flags.
        if flags.has(MovementFlags::FIXED_Z) {
            cheats |= cheat::FIXED_Z;
            remove |= MovementFlags::FIXED_Z;
        }
        if flags.has(MovementFlags::LEVITATING) {
            cheats |= cheat::FLY_HACK_SWIM;
            remove |= MovementFlags::LEVITATING;
        }
        if flags.has(MovementFlags::SWIMMING) && flags.has(MovementFlags::FLYING) {
            cheats |= cheat::FLY_HACK_SWIM;
            remove |= MovementFlags::SWIMMING | MovementFlags::FLYING;
        }
        if flags.has(MovementFlags::WATER_WALKING)
            && !last.flags.has(MovementFlags::WATER_WALKING)
            && !facts.has_water_walk_aura
            && !facts.has_ghost_aura
            && !facts.pending_water_walk
            && opcode != op::CMSG_MOVE_WATER_WALK_ACK
        {
            cheats |= cheat::WATER_WALK;
            remove |= MovementFlags::WATER_WALKING;
        }
        if flags.has(MovementFlags::SAFE_FALL)
            && !last.flags.has(MovementFlags::SAFE_FALL)
            && !facts.has_feather_fall_aura
            && !facts.pending_feather_fall
            && opcode != op::CMSG_MOVE_FEATHER_FALL_ACK
        {
            cheats |= cheat::SLOW_FALL;
            remove |= MovementFlags::SAFE_FALL;
        }
        if flags.has(MovementFlags::HOVER)
            && !last.flags.has(MovementFlags::HOVER)
            && !facts.has_hover_aura
            && !facts.pending_hover
            && opcode != op::CMSG_MOVE_HOVER_ACK
        {
            cheats |= cheat::HOVER;
            remove |= MovementFlags::HOVER;
        }
        // Per-cheat Enable gates counting (fork gates
        // in ComputeCheatAction; detection still strips).
        if cheats == 0 {
            return (MoveVerdict::Accept, 0);
        }
        if opcode == op::CMSG_MOVE_SPLINE_DONE {
            // Ignore packet, no forced update.
            return (MoveVerdict::Reject, 0);
        }
        self.store(cheats);
        if cfg.should_reject(cheats) && facts.spline_finalized && !facts.being_teleported {
            return (MoveVerdict::Reject, remove);
        }
        if remove != 0 {
            info.flags.remove(remove);
        }
        (MoveVerdict::Accept, cheats)
    }

    /// `HandlePositionTests`.
    #[allow(clippy::too_many_lines)] // one HandlePositionTests port
    pub fn position_tests(
        &mut self,
        cfg: &AnticheatConfig,
        facts: &MoveFacts,
        last: &MovementInfo,
        info: &MovementInfo,
        opcode: u16,
        session_guid: u32,
    ) -> (MoveVerdict, u32) {
        use mangos_protocol::opcode_table::opcode_id as op;
        if !cfg.enabled || (cfg.players_only && facts.security != 0) || !facts.spline_finalized {
            return (MoveVerdict::Accept, 0);
        }
        let mut cheats = 0u32;
        if cfg.cheat_cfg(cheat::NULL_CLIENT_TIME).enable && info.client_time == 0 {
            cheats |= cheat::NULL_CLIENT_TIME;
        }
        let jumping = MovementFlags::JUMPING | MovementFlags::FALLING_FAR;
        // Jump speed change mid-jump (build > 1.9.4 arm).
        if cfg.cheat_cfg(cheat::JUMP_SPEED_CHANGE).enable
            && info.flags.has(jumping)
            && last.flags.has(jumping)
            && info.jump.xy_speed > last.jump.xy_speed + 0.0001
            && last.jump.xy_speed > 0.0001
            && !self.knock_back
        {
            cheats |= cheat::JUMP_SPEED_CHANGE;
        }
        if cfg.cheat_cfg(cheat::OVERSPEED_JUMP).enable
            && opcode == op::MSG_MOVE_JUMP
            && info.jump.xy_speed > facts.speeds[move_type_for(last)] + 0.0001
        {
            cheats |= cheat::OVERSPEED_JUMP;
        }
        // Multi-jump.
        if cfg.cheat_cfg(cheat::MULTI_JUMP).enable {
            match opcode {
                op::MSG_MOVE_JUMP => {
                    self.jump_count += 1;
                    if self.jump_count > 1 {
                        cheats |= cheat::MULTI_JUMP;
                    }
                }
                op::MSG_MOVE_FALL_LAND | op::MSG_MOVE_START_SWIM => {
                    self.jump_count = 0;
                }
                _ => {}
            }
        }
        if opcode == op::MSG_MOVE_STOP_SWIM && info.flags.has(MovementFlags::SWIMMING) {
            cheats |= cheat::FLY_HACK_SWIM;
        }
        // Time desync / reverse time.
        if was_sent_by_session(last, session_guid) {
            if last.flags.has(MovementFlags::MASK_MOVING) {
                #[allow(clippy::cast_possible_wrap)]
                let current = (info.client_time.wrapping_sub(last.client_time) as i32)
                    - (info.server_time.wrapping_sub(last.server_time) as i32);
                self.client_desync += current;
                if cfg.cheat_cfg(cheat::NUM_DESYNC).enable && current > 1000 {
                    cheats |= cheat::NUM_DESYNC;
                }
            }
            if cfg.cheat_cfg(cheat::TIME_BACK).enable && info.client_time < last.client_time {
                cheats |= cheat::TIME_BACK;
            }
        }
        // Teleport (distance from server position +
        // axis movement without flags).
        let mut teleport = false;
        if cfg.cheat_cfg(cheat::TELEPORT).enable && !facts.being_teleported {
            teleport = Self::check_teleport(cfg, facts, last, info);
            if teleport {
                cheats |= cheat::TELEPORT;
            }
        }
        // Fake transport / teleport to transport.
        let transport_rise = info.flags.has(MovementFlags::ON_TRANSPORT)
            && last.client_time != 0
            && !last.flags.has(MovementFlags::ON_TRANSPORT);
        if cfg.cheat_cfg(cheat::FAKE_TRANSPORT).enable
            && transport_rise
            && !facts.claimed_transport_ok
        {
            cheats |= cheat::FAKE_TRANSPORT;
        }
        if cfg.cheat_cfg(cheat::TELEPORT_TRANSPORT).enable && transport_rise {
            let d2 = dist2d((last.pos.x, last.pos.y), (info.pos.x, info.pos.y));
            if d2 > 100.0 {
                cheats |= cheat::TELEPORT_TRANSPORT;
            }
        }
        // Wall climb (high-slope arm; mid arm needs a
        // vmap/static height split the Rust terrain
        // layer does not expose).
        if cfg.cheat_cfg(cheat::WALL_CLIMB).enable && self.check_wall_climb(cfg, facts, last, info)
        {
            cheats |= cheat::WALL_CLIMB;
        }
        // Fall-time family.
        if cfg.cheat_cfg(cheat::NO_FALL_TIME).enable
            && self.check_no_fall_time(last, info, opcode, session_guid)
        {
            cheats |= cheat::NO_FALL_TIME;
        }
        if cfg.cheat_cfg(cheat::BAD_FALL_RESET).enable
            && opcode == op::CMSG_MOVE_FALL_RESET
            && !info.flags.has(MovementFlags::FIXED_Z)
            && ((was_sent_by_session(last, session_guid) && !last.flags.has(jumping))
                || info.fall_time != 0
                || info.jump.z_speed != 0.0)
        {
            cheats |= cheat::BAD_FALL_RESET;
        }
        if cfg.cheat_cfg(cheat::BAD_FALL_STOP).enable
            && was_sent_by_session(last, session_guid)
            && opcode != op::MSG_MOVE_FALL_LAND
            && opcode != op::MSG_MOVE_START_SWIM
            && opcode != op::CMSG_FORCE_MOVE_ROOT_ACK
            && opcode != op::CMSG_MOVE_NOT_ACTIVE_MOVER
            && !info.flags.has(MovementFlags::ROOT)
            && !info.flags.has(jumping)
            && last.flags.has(jumping)
        {
            cheats |= cheat::BAD_FALL_STOP;
        }
        if cfg.cheat_cfg(cheat::BAD_MOVE_START).enable
            && Self::check_move_start(last, info, opcode, session_guid)
        {
            cheats |= cheat::BAD_MOVE_START;
        }
        // Speed hack (skipped on teleport, fork order).
        if !teleport {
            cheats |= self.check_speed_hack(cfg, facts, last, info, opcode);
        }
        if opcode == op::MSG_MOVE_FALL_LAND
            || opcode == op::MSG_MOVE_START_SWIM
            || info.flags.has(MovementFlags::ROOT)
        {
            self.knock_back = false;
        }
        self.store(cheats);
        if cfg.should_reject(cheats) {
            if !last.flags.has(jumping) {
                self.reset_jump_counters();
            }
            return (MoveVerdict::Reject, cheats);
        }
        (MoveVerdict::Accept, cheats)
    }

    fn check_teleport(
        cfg: &AnticheatConfig,
        facts: &MoveFacts,
        last: &MovementInfo,
        info: &MovementInfo,
    ) -> bool {
        // IsTeleportAllowed3D.
        if !facts.on_transport_server && !info.flags.has(MovementFlags::ON_TRANSPORT) {
            let d = ((facts.server_pos.0 - info.pos.x).powi(2)
                + (facts.server_pos.1 - info.pos.y).powi(2)
                + (facts.server_pos.2 - info.pos.z).powi(2))
            .sqrt();
            let max =
                cfg.teleport_distance * (facts.speed_rates[move_type_for(info)] * 0.2).max(1.0);
            if d > max {
                if facts.in_transport_area {
                    if d >= ALLOWED_TRANSPORT_DISTANCE {
                        return true;
                    }
                } else {
                    return true;
                }
            }
        }
        // Axis movement without the matching flags.
        let xz = MovementFlags::MASK_XZ | MovementFlags::JUMPING | MovementFlags::FALLING_FAR;
        if last.client_time != 0
            && !last.flags.has(xz)
            && !info.flags.has(xz)
            && last.flags.has(MovementFlags::ON_TRANSPORT)
                == info.flags.has(MovementFlags::ON_TRANSPORT)
        {
            let on_t = info.flags.has(MovementFlags::ON_TRANSPORT);
            let (a, b) = if on_t {
                (
                    (last.transport_pos.x, last.transport_pos.y),
                    (info.transport_pos.x, info.transport_pos.y),
                )
            } else {
                ((last.pos.x, last.pos.y), (info.pos.x, info.pos.y))
            };
            if dist2d(a, b) > 1.0 {
                return true;
            }
            if !last.flags.has(MovementFlags::SWIMMING) && !info.flags.has(MovementFlags::SWIMMING)
            {
                let dz = if on_t {
                    (last.transport_pos.z - info.transport_pos.z).abs()
                } else {
                    (last.pos.z - info.pos.z).abs()
                };
                if dz > 2.0 {
                    return true;
                }
            }
        }
        false
    }

    fn check_wall_climb(
        &self,
        cfg: &AnticheatConfig,
        facts: &MoveFacts,
        last: &MovementInfo,
        info: &MovementInfo,
    ) -> bool {
        let no_check = MovementFlags::JUMPING
            | MovementFlags::FALLING_FAR
            | MovementFlags::SWIMMING
            | MovementFlags::FLYING
            | MovementFlags::PITCH_UP
            | MovementFlags::PITCH_DOWN
            | MovementFlags::ON_TRANSPORT
            | MovementFlags::SPLINE_ELEVATION;
        if last.flags.has(no_check)
            || info.flags.has(no_check)
            || self.knock_back
            || facts.being_teleported
            || last.client_time == 0
        {
            return false;
        }
        let delta_xy = dist2d((last.pos.x, last.pos.y), (info.pos.x, info.pos.y));
        if delta_xy < 0.5 {
            return false;
        }
        let delta_z = info.pos.z - last.pos.z;
        if delta_z < 1.0 {
            return false;
        }
        delta_z > cfg.wall_slope_high * delta_xy
    }

    fn check_no_fall_time(
        &mut self,
        last: &MovementInfo,
        info: &MovementInfo,
        opcode: u16,
        session_guid: u32,
    ) -> bool {
        use mangos_protocol::opcode_table::opcode_id as op;
        let jumping = MovementFlags::JUMPING | MovementFlags::FALLING_FAR;
        if opcode == op::CMSG_MOVE_FALL_RESET || info.flags.has(MovementFlags::FIXED_Z) {
            self.jump_flag_count = 0;
            self.jump_flag_time = 0;
            return false;
        }
        if !was_sent_by_session(last, session_guid) || !last.flags.has(jumping) {
            self.jump_flag_count = 0;
            self.jump_flag_time = 0;
        }
        if info.flags.has(jumping) {
            self.jump_flag_count += 1;
            if self.jump_flag_time == 0 {
                self.jump_flag_time = info.client_time;
            }
        }
        if info.fall_time == 0
            && self.jump_flag_count >= JUMP_FLAG_THRESHOLD
            && self.jump_flag_time + MIN_FALLING_TIME < info.client_time
        {
            self.jump_flag_count = 0;
            self.jump_flag_time = 0;
            return true;
        }
        if !info.flags.has(jumping) {
            self.jump_flag_count = 0;
            self.jump_flag_time = 0;
        }
        false
    }

    fn check_move_start(
        last: &MovementInfo,
        info: &MovementInfo,
        opcode: u16,
        session_guid: u32,
    ) -> bool {
        use mangos_protocol::opcode_table::opcode_id as op;
        if !was_sent_by_session(last, session_guid) {
            return false;
        }
        if opcode == op::MSG_MOVE_FALL_LAND || opcode == op::MSG_MOVE_START_SWIM {
            return false;
        }
        // Any move-ack opcodes are dispatched to the
        // ACK family in this server, never reaching
        // here; the fork exemption is thus structural.
        let pairs: [(u16, u32); 8] = [
            (op::MSG_MOVE_START_FORWARD, MovementFlags::FORWARD),
            (op::MSG_MOVE_START_BACKWARD, MovementFlags::BACKWARD),
            (op::MSG_MOVE_START_PITCH_DOWN, MovementFlags::PITCH_DOWN),
            (op::MSG_MOVE_START_PITCH_UP, MovementFlags::PITCH_UP),
            (op::MSG_MOVE_START_STRAFE_LEFT, MovementFlags::STRAFE_LEFT),
            (op::MSG_MOVE_START_STRAFE_RIGHT, MovementFlags::STRAFE_RIGHT),
            (op::MSG_MOVE_START_SWIM, MovementFlags::SWIMMING),
            (op::MSG_MOVE_START_TURN_LEFT, MovementFlags::TURN_LEFT),
        ];
        for (start_op, flag) in pairs {
            if opcode == start_op {
                if !info.flags.has(flag) {
                    return true;
                }
            } else if info.flags.has(flag) && !last.flags.has(flag) {
                return true;
            }
        }
        // Turn-right pair (table capped at 8 for the
        // swimming special-case parity with fork).
        if opcode == op::MSG_MOVE_START_TURN_RIGHT {
            if !info.flags.has(MovementFlags::TURN_RIGHT) {
                return true;
            }
        } else if info.flags.has(MovementFlags::TURN_RIGHT)
            && !last.flags.has(MovementFlags::TURN_RIGHT)
        {
            return true;
        }
        false
    }

    fn check_speed_hack(
        &mut self,
        cfg: &AnticheatConfig,
        facts: &MoveFacts,
        last: &MovementInfo,
        info: &MovementInfo,
        opcode: u16,
    ) -> u32 {
        use mangos_protocol::opcode_table::opcode_id as op;
        if opcode == op::CMSG_MOVE_KNOCK_BACK_ACK
            || opcode == op::CMSG_MOVE_SPLINE_DONE
            || self.knock_back
            || last.client_time == 0
            || facts.being_teleported
        {
            return 0;
        }
        #[allow(clippy::cast_possible_wrap)]
        let mut client_diff = info.client_time.wrapping_sub(last.client_time) as i32;
        if cfg.max_allowed_desync != 0 && client_diff > cfg.max_allowed_desync {
            client_diff = cfg.max_allowed_desync;
        }
        if cfg.speed_hack_enable {
            if let Some((ix, iy, iz)) = facts.extrapolated {
                let dx = ix - last.pos.x;
                let dy = iy - last.pos.y;
                let allowed_sq = dx * dx + dy * dy;
                let real_sq = (info.pos.x - last.pos.x).powi(2) + (info.pos.y - last.pos.y).powi(2);
                if real_sq > allowed_sq * 1.1 {
                    self.overspeed_distance += real_sq.sqrt() - allowed_sq.sqrt();
                }
                let _ = (ix, iy, iz);
            } else if !info.transport_guid.is_empty()
                && info.flags.has(MovementFlags::ON_TRANSPORT)
                && !last.transport_guid.is_empty()
                && last.flags.has(MovementFlags::ON_TRANSPORT)
            {
                let traveled = dist2d(
                    (last.transport_pos.x, last.transport_pos.y),
                    (info.transport_pos.x, info.transport_pos.y),
                );
                #[allow(clippy::cast_precision_loss)]
                let time_factor = 1000.0f32 / i32_max1(client_diff) as f32;
                if traveled > 0.0 {
                    let allowed = (facts.speeds[move_type_for(info)] / time_factor)
                        .max(facts.speeds[move_type_for(last)] / time_factor);
                    if traveled > allowed * 2.0 {
                        self.overspeed_distance += traveled - allowed;
                    }
                }
            }
        }
        if client_diff > 1000 && last.flags.has(MovementFlags::MASK_MOVING) {
            return cheat::SKIPPED_HEARTBEATS;
        }
        0
    }
}

fn i32_max1(v: i32) -> i32 {
    v.max(1)
}

/// Extrapolate `last` over `diff_ms` of client time
/// (`Unit::ExtrapolateMovement` non-spline arm; the
/// caller applies ground snapping/LOS policy — here
/// ground snap via `ground_z`, LOS unported).
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::similar_names,
    clippy::many_single_char_names
)] // fork ExtrapolateMovement math
pub fn extrapolate(
    last: &MovementInfo,
    diff_ms: u32,
    speeds: &[f32; 6],
    jump_initial_speed: f32,
    mut ground_z: impl FnMut(f32, f32, f32) -> f32,
) -> Option<(f32, f32, f32)> {
    let flags = last.flags;
    if flags.has(
        MovementFlags::PITCH_UP
            | MovementFlags::PITCH_DOWN
            | MovementFlags::FALLING_FAR
            | MovementFlags::ON_TRANSPORT,
    ) || last.client_time == 0
    {
        return None;
    }
    let (mut x, mut y, mut z) = (last.pos.x, last.pos.y, last.pos.z);
    let mut o = last.pos.o;
    if flags.has(MovementFlags::ROOT) {
        return Some((x, y, z));
    }
    let speed = speeds[move_type_for(last)];
    if flags.has(MovementFlags::BACKWARD) {
        o += std::f32::consts::PI;
    } else if flags.has(MovementFlags::STRAFE_LEFT) {
        o += if flags.has(MovementFlags::FORWARD) {
            std::f32::consts::FRAC_PI_4
        } else {
            std::f32::consts::FRAC_PI_2
        };
    } else if flags.has(MovementFlags::STRAFE_RIGHT) {
        o -= if flags.has(MovementFlags::FORWARD) {
            std::f32::consts::FRAC_PI_4
        } else {
            std::f32::consts::FRAC_PI_2
        };
    }
    #[allow(clippy::cast_precision_loss)]
    let diff_s = diff_ms as f32 / 1000.0;
    if flags.has(MovementFlags::JUMPING) {
        let jump: JumpInfo = last.jump;
        #[allow(clippy::cast_precision_loss)]
        let diff_t =
            (last.client_time + diff_ms).wrapping_sub(jump.start_client_time) as f32 / 1000.0;
        x = jump.start.x;
        y = jump.start.y;
        z = jump.start.z;
        if x == 0.0 || y == 0.0 || z == 0.0 || diff_t > 10000.0 {
            return None;
        }
        x += jump.cos_angle * jump.xy_speed * diff_t;
        y += jump.sin_angle * jump.xy_speed * diff_t;
        z -= compute_fall_elevation(
            diff_t,
            flags.has(MovementFlags::SAFE_FALL),
            -jump_initial_speed,
        );
    } else if flags.has(MovementFlags::TURN_LEFT | MovementFlags::TURN_RIGHT)
        && flags.has(MovementFlags::MASK_MOVING)
    {
        let turn_rate = speeds[5];
        let mut t = 0.75 * turn_rate * diff_s;
        let r = 1.295 * speed / std::f32::consts::PI * last.swim_pitch.cos();
        z += diff_s * speed * last.swim_pitch.sin();
        if flags.has(MovementFlags::TURN_LEFT) {
            x += r * (o + std::f32::consts::FRAC_PI_2).cos();
            y += r * (o + std::f32::consts::FRAC_PI_2).sin();
            t -= std::f32::consts::FRAC_PI_2;
        } else {
            x += r * (o - std::f32::consts::FRAC_PI_2).cos();
            y += r * (o - std::f32::consts::FRAC_PI_2).sin();
            t = -t + std::f32::consts::FRAC_PI_2;
        }
        x += r * (o + t).cos();
        y += r * (o + t).sin();
    } else if flags.has(MovementFlags::MASK_MOVING) {
        let dist = speed * diff_s;
        x += dist * o.cos() * last.swim_pitch.cos();
        y += dist * o.sin() * last.swim_pitch.cos();
        z += dist * last.swim_pitch.sin();
    } else {
        return Some((x, y, z));
    }
    if !mangos_movement::movement_info::is_valid_map_coord(x, y, z, o) {
        return None;
    }
    if !flags.has(MovementFlags::JUMPING | MovementFlags::FALLING_FAR | MovementFlags::SWIMMING) {
        z = ground_z(x, y, z);
    }
    Some((x, y, z))
}
