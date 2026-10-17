//! Rust `mangosd` world server (migration document section 12:
//! Tokio runtime + world actor + per-map executors).
//!
//! Phase 3 scope: the boot/session skeleton — listener, socket-layer
//! auth and ping, a single world actor at the C++ 50 ms tick, and
//! class-stubbed opcode dispatch over the generated 828-slot table.
//! World simulation arrives in later phases; nothing here invents
//! behaviour the C++ does not have.

pub mod account_data;
pub mod addon_info;
pub mod auth;
pub mod char_enum;
pub mod char_queries;
pub mod ping;
pub mod player_login;
pub mod session_core;
pub mod socket;
pub mod world_actor;

use std::net::{IpAddr, Ipv4Addr};

use mangos_config::Config;

/// `SHUTDOWN_EXIT_CODE` (`World.h:77`): clean shutdown.
pub const SHUTDOWN_EXIT_CODE: u8 = 0;
/// `ERROR_EXIT_CODE` (`World.h:78`).
pub const ERROR_EXIT_CODE: u8 = 1;
/// `RESTART_EXIT_CODE` (`World.h:79`): what SIGINT maps to
/// (`Master.cpp:522-523`). Load-bearing for the supervisor.
pub const RESTART_EXIT_CODE: u8 = 2;

/// `EXPECTED_MANGOSD_CLIENT_BUILD` for the 1.12.1 progression gate
/// (`cmake/generators/Progression.h.in:40`).
pub const ACCEPTED_CLIENT_BUILDS: [u32; 3] = [5875, 6005, 6141];

/// `IsAcceptableClientBuild` (`DBCStores.cpp:117-125`).
#[must_use]
pub fn is_acceptable_client_build(build: u32) -> bool {
    ACCEPTED_CLIENT_BUILDS.contains(&build)
}

/// Security levels from `enum AccountTypes` (`Common.h:136-145`),
/// kept as the raw u8 the DB stores.
pub mod security {
    pub const PLAYER: u8 = 0;
    pub const ADMINISTRATOR: u8 = 6;
}

/// The `mangosd.conf` keys the Phase 3 skeleton consumes. Defaults
/// mirror the C++ `setConfig` calls and `GetIntDefault` fallbacks,
/// not the `.dist` file's explicit values, so an absent key behaves
/// the same as in C++.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// `RealmID`; 0/missing fails boot (`Master.cpp:461-466`).
    pub realm_id: u32,
    /// `WorldServerPort` (default 8085).
    pub port: u16,
    /// `BindIP` (default `0.0.0.0`).
    pub bind_ip: IpAddr,
    /// `MaxOverspeedPings` (default 2; 0 disables; 1 clamps to 2,
    /// `World.cpp:703-708`).
    pub max_overspeed_pings: u32,
    /// `Network.KickOnBadPacket` (default false, `World.cpp:1055`).
    pub kick_on_bad_packet: bool,
    /// `Network.TimeoutSecsIfNoAuth` (default 10, 0 disables;
    /// `WorldSocket.cpp:594`).
    pub no_auth_timeout_secs: u32,
    /// `CharacterScreenMaxIdleTime` seconds (C++ code default 0 =
    /// disabled, `World.cpp:555`; the `.dist` file sets 900).
    pub character_screen_max_idle_secs: u32,
    /// Security level floor for auth, derived from `PlayerLimit`
    /// exactly like `World::GetPlayerSecurityLimit` (`World.h:705`):
    /// a non-positive limit `-n` demands security `n`; any positive
    /// limit is a plain player count and imposes no floor.
    /// `PlayerLimit` defaults to 100 (`SystemConfig.h:92`,
    /// `World.cpp:467`).
    pub player_security_limit: u8,
    /// `Antiflood.Sanction` raw bitmask (default
    /// `CHEAT_ACTION_KICK = 0x08`, `World.cpp:804`,
    /// `Anticheat.h:34-41`). Phase 3 honors the kick bit and logs
    /// the rest; the full anticheat action lands later.
    pub antiflood_sanction: u32,
    /// Raw `PlayerLimit` (default 100): positive = admission
    /// cap (login queue beyond it), non-positive encodes the
    /// security floor in `player_security_limit`.
    pub player_limit: i32,
    /// `LoginQueue.GracePeriodSecs` (default 0, `World.cpp:554`).
    pub login_queue_grace_secs: u32,
    /// `RestrictUnverifiedAccounts` (default false,
    /// `World.cpp:614`); drives the trial billing flags.
    pub restrict_unverified_accounts: bool,
    /// `WorldAvailable` (default true, `World.cpp:609`); when
    /// false, plain players get `SMSG_CHARACTER_LOGIN_FAILED`.
    pub world_available: bool,
    /// `WowPatch` (default 0 = patch 1.2, clamped to 10 at the
    /// 1.12 gate, `World.cpp:461-465`).
    pub wow_patch: u32,
    /// `UpdateUptimeInterval` in minutes (default 10,
    /// `World.cpp:679`). Zero disables the periodic refresh
    /// (the C++ timer would fire every tick at 0; recorded
    /// deviation, dist default is 10).
    pub uptime_update_mins: u32,
    /// `Motd` base text; the patch banner is appended at load
    /// (`World.cpp:467`).
    pub motd: String,
    /// `GameType` (`RealmType`, default 0 = NORMAL,
    /// `World.cpp:582-583`). `FFA_PVP` (16) is published as `PVP` (1).
    pub game_type: u32,
    /// `RealmZone` (default 1 = DEVELOPMENT, `World.cpp:585-586`).
    pub realm_zone: u32,
}

impl ServerConfig {
    #[must_use]
    pub fn from_config(cfg: &Config) -> Self {
        let mut max_overspeed_pings = to_u32(cfg.int_default("MaxOverspeedPings", 2));
        if max_overspeed_pings == 1 {
            max_overspeed_pings = 2;
        }
        let player_limit = cfg.int_default("PlayerLimit", 100);
        let player_security_limit = if player_limit <= 0 {
            i64::from(-player_limit).min(i64::from(security::ADMINISTRATOR)) as u8
        } else {
            security::PLAYER
        };
        ServerConfig {
            realm_id: to_u32(cfg.int_default("RealmID", 0)),
            port: to_u32(cfg.int_default("WorldServerPort", 8085)) as u16,
            bind_ip: cfg
                .string_default("BindIP", "0.0.0.0")
                .parse()
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            max_overspeed_pings,
            kick_on_bad_packet: cfg.bool_default("Network.KickOnBadPacket", false),
            no_auth_timeout_secs: to_u32(cfg.int_default("Network.TimeoutSecsIfNoAuth", 10)),
            character_screen_max_idle_secs: to_u32(
                cfg.int_default("CharacterScreenMaxIdleTime", 0),
            ),
            player_security_limit,
            antiflood_sanction: to_u32(cfg.int_default("Antiflood.Sanction", 8)),
            player_limit,
            login_queue_grace_secs: to_u32(cfg.int_default("LoginQueue.GracePeriodSecs", 0)),
            restrict_unverified_accounts: cfg.bool_default("RestrictUnverifiedAccounts", false),
            world_available: cfg.bool_default("WorldAvailable", true),
            wow_patch: to_u32(cfg.int_default("WowPatch", 0)).min(10),
            uptime_update_mins: to_u32(cfg.int_default("UpdateUptimeInterval", 10)),
            motd: cfg.string_default("Motd", "Welcome to the Massive Network Game Object Server."),
            game_type: to_u32(cfg.int_default("GameType", 0)),
            realm_zone: to_u32(cfg.int_default("RealmZone", 1)),
        }
    }
}

/// C++ int configs are reinterpreted as unsigned at use sites;
/// negative values wrap. Centralised so the cast is reviewed once.
const fn to_u32(value: i32) -> u32 {
    value as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(text: &str) -> Config {
        Config::parse(text.as_bytes())
    }

    #[test]
    fn defaults_match_cpp_code() {
        let s = ServerConfig::from_config(&cfg("RealmID = 1\n"));
        assert_eq!(s.realm_id, 1);
        assert_eq!(s.port, 8085);
        assert_eq!(s.max_overspeed_pings, 2);
        assert!(!s.kick_on_bad_packet);
        assert_eq!(s.no_auth_timeout_secs, 10);
        assert_eq!(s.character_screen_max_idle_secs, 0);
        assert_eq!(s.player_security_limit, security::PLAYER);
    }

    #[test]
    fn overspeed_clamp_and_security_floor() {
        let s = ServerConfig::from_config(&cfg("MaxOverspeedPings = 1\nPlayerLimit = -3\n"));
        assert_eq!(s.max_overspeed_pings, 2);
        assert_eq!(s.player_security_limit, 3);
        let s = ServerConfig::from_config(&cfg("MaxOverspeedPings = 0\nPlayerLimit = 50\n"));
        assert_eq!(s.max_overspeed_pings, 0);
        assert_eq!(s.player_security_limit, security::PLAYER);
    }

    #[test]
    fn accepted_builds() {
        assert!(is_acceptable_client_build(5875));
        assert!(is_acceptable_client_build(6141));
        assert!(!is_acceptable_client_build(5464));
        assert!(!is_acceptable_client_build(0));
    }
}
