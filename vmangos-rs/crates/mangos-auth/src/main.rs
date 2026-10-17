//! `realmd` — the Rust auth/logon server, a behaviour port of
//! `vmangos/src/realmd` (`Main.cpp`, `AuthSocket.cpp`, `RealmList.cpp`)
//! at production commit `2bae5f9` (HEAD verified formatting-only for
//! these files; see `docs/rust-migration-evidence/PHASE1-PROGRESS.md`).
//!
//! Threading: one thread per connection with blocking reads, mirroring
//! the C++ async handler chain's observable wire behaviour; all login-DB
//! work serializes through the single `LoginDb` actor.
//!
//! Deliberate Phase 1 deviations (recorded in PHASE1-PROGRESS.md):
//! PROXY protocol v2, geolock/geoip enforcement and its PIN writes,
//! `ENABLE_MAILSENDER` mail, `-s run` daemonization (runs foreground),
//! CPU affinity/priority, and DB-error-to-client mapping (C++ trusts
//! its DB layer; a driver error here closes the session instead of
//! fabricating a reply the C++ would not send).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use mangos_config::Config;
use mangos_db::LoginDb;
use mangos_db::login::RealmlistRow;
use mangos_log::{Level, Log};
use mangos_protocol::auth::{
    LogonChallengeBody, PinGrid, VERSION_CHALLENGE, build_challenge_failure,
    build_logon_challenge_success, build_logon_failure, build_logon_proof_success, cmd,
    parse_logon_challenge_body, parse_logon_challenge_header, parse_logon_proof,
    parse_reconnect_proof, result,
};
use mangos_protocol::buffer::{PacketReader, PacketWriter};
use mangos_protocol::srp::{
    G, SrpSession, bn_as_byte_array, bn_as_byte_array_padded, bn_from_binary, bn_from_hex, prime_n,
    verify_reconnect_proof,
};
use num_bigint::BigUint;
use sha1::{Digest, Sha1};

const REALMD_CONF_VERSION: i32 = 2_024_091_701;
const DEFAULT_CONFIG_PATH: &str = "realmd.conf";
const DEFAULT_REALMSERVER_PORT: i32 = 3724;
const MAX_USERNAME_LEN: usize = 16;
/// `sizeof(sAuthLogonChallengeBody) - AUTH_LOGON_MAX_NAME` = 46 - 16.
const MIN_CHALLENGE_BODY: usize = 30;
const SEC_ADMINISTRATOR: u8 = 6;

const LOCK_IP_LOCK: u8 = 0x01;
const LOCK_FIXED_PIN: u8 = 0x02;
const LOCK_TOTP: u8 = 0x04;
const LOCK_ALWAYS_ENFORCE: u8 = 0x08;

const REALM_FLAG_OFFLINE: u8 = 0x02;
const REALM_FLAG_SPECIFYBUILD: u8 = 0x04;
const REALM_FLAG_NEW_PLAYERS: u8 = 0x20;
const REALM_FLAG_RECOMMENDED: u8 = 0x40;
const REALM_FLAGS_ALLOWED_MASK: u8 =
    REALM_FLAG_OFFLINE | REALM_FLAG_NEW_PLAYERS | REALM_FLAG_RECOMMENDED | REALM_FLAG_SPECIFYBUILD;

/// `MIGRATIONS_LOGON` (generated from `vmangos/sql/migrations/*_logon.sql`).
const REQUIRED_LOGON_MIGRATIONS: [&str; 6] = [
    "20210830151515",
    "20220826100652",
    "20221111031829",
    "20221117065844",
    "20240107103630",
    "20260109115717",
];

const MAX_REALM_ZONES: usize = 38;
/// `RealmCategoryIdsByRealmZoneByMajorVersion` (`RealmList.cpp:70-75`).
#[rustfmt::skip]
const REALM_CATEGORY_IDS: [[u8; MAX_REALM_ZONES]; 4] = [
    [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 1, 5, 1, 1, 1, 1, 1, 2, 3, 5, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37],
];

/// `GetLocaleByName` table (`Common.cpp:28-40`); unknown names map to 0.
fn locale_by_name(name: &str) -> u32 {
    match name {
        "koKR" => 1,
        "frFR" => 2,
        "deDE" => 3,
        "zhCN" => 4,
        "zhTW" => 5,
        "esES" => 6,
        "esMX" => 7,
        "ruRU" => 8,
        _ => 0, // enUS / enGB / unknown
    }
}

/// One `allowed_clients` row (`RealmBuildInfo`).
#[derive(Clone)]
struct RealmBuildInfo {
    major_version: u8,
    minor_version: u8,
    bugfix_version: u8,
    _hotfix_version: u8,
    build: u16,
    os: String,
    platform: String,
    integrity_hash: [u8; 20],
}

/// One realm as held by `RealmList` (keyed by name in the map).
#[derive(Clone)]
struct Realm {
    id: u32,
    icon: u8,
    flags: u8,
    timezone: u8,
    allowed_security_level: u8,
    population: f32,
    builds: BTreeSet<u32>,
    build_info: RealmBuildInfo,
    external_ip: Ipv4Addr,
    local_ip: Ipv4Addr,
    local_cidr: u8,
    port: u16,
}

impl Realm {
    /// `Realm::GetAddressForClient` (`RealmList.cpp:285`): the local
    /// endpoint when the client shares the realm's local subnet.
    fn address_for_client(&self, client: IpAddr) -> String {
        let local = match client {
            IpAddr::V4(v4) => is_in_same_subnet(v4, self.local_ip, self.local_cidr),
            IpAddr::V6(_) => false,
        };
        let ip = if local {
            self.local_ip
        } else {
            self.external_ip
        };
        format!("{}:{}", ip, self.port)
    }
}

/// Mask from CIDR prefix length (C++ `IsInSameSubnet` over IPv4 words).
fn is_in_same_subnet(client: Ipv4Addr, local: Ipv4Addr, cidr: u8) -> bool {
    if cidr == 0 {
        return true;
    }
    if cidr >= 32 {
        return client == local;
    }
    let mask = u32::MAX << (32 - u32::from(cidr));
    (u32::from(client) & mask) == (u32::from(local) & mask)
}

/// `FindBuildInfo(uint16)` (`RealmList.cpp:55-68`): the first row is the
/// low bound of an always-accepted range. Empty list is guarded at
/// startup (C++ indexes `[0]` unconditionally).
fn find_build_info(builds: &[RealmBuildInfo], build: u16) -> Option<&RealmBuildInfo> {
    let first = builds.first()?;
    if build >= first.build {
        return Some(first);
    }
    builds.iter().skip(1).find(|b| b.build == build)
}

/// `FindBuildInfo(uint16, os, platform)` (`RealmList.cpp:42-53`).
fn find_build_info_full<'a>(
    builds: &'a [RealmBuildInfo],
    build: u16,
    os: &str,
    platform: &str,
) -> Vec<&'a RealmBuildInfo> {
    builds
        .iter()
        .filter(|b| b.build == build && b.os == os && b.platform == platform)
        .collect()
}

/// `GetRealmCategoryIdByBuildAndZone` (`RealmList.cpp:77-88`).
fn realm_category_id(builds: &[RealmBuildInfo], build: u16, zone: u8) -> u8 {
    let zone_idx = if usize::from(zone) >= MAX_REALM_ZONES {
        0 // REALM_ZONE_DEVELOPMENT
    } else {
        usize::from(zone)
    };
    match find_build_info(builds, build) {
        Some(info) if info.major_version < 4 => {
            REALM_CATEGORY_IDS[usize::from(info.major_version)][zone_idx]
        }
        _ => zone_idx as u8,
    }
}

/// Mutable realm/build snapshot, refreshed by `UpdateIfNeed` semantics.
struct RealmState {
    realms: BTreeMap<String, Realm>,
    builds: Vec<RealmBuildInfo>,
    update_interval_secs: u64,
    next_update: Instant,
}

impl RealmState {
    /// `RealmList::Initialize` + `LoadAllowedClients` + `UpdateRealms(true)`.
    fn initialize(server: &Server, update_interval_secs: u64) -> Option<RealmState> {
        let builds = load_allowed_clients(server)?;
        let mut state = RealmState {
            realms: BTreeMap::new(),
            builds,
            update_interval_secs,
            next_update: Instant::now(),
        };
        state.update_realms(server, true)?;
        Some(state)
    }

    /// `RealmList::UpdateIfNeed`: reloads realms (not builds) when due.
    fn update_if_need(&mut self, server: &Server) {
        if self.update_interval_secs == 0 || self.next_update > Instant::now() {
            return;
        }
        self.next_update = Instant::now() + Duration::from_secs(self.update_interval_secs);
        self.realms.clear();
        let _ = self.update_realms(server, false);
    }

    /// `RealmList::UpdateRealms`: parse/validate each row, skipping bad
    /// ones with the C++ error lines. Returns false on DB failure.
    fn update_realms(&mut self, server: &Server, init: bool) -> Option<()> {
        server.out(Level::Detail, "Updating realm list...");
        let rows = server.db.realmlist().ok()?;
        for row in rows {
            let Some(realm) = parse_realm_row(server, &self.builds, &row) else {
                continue;
            };
            if init {
                server.out(Level::Minimal, &format!("Added realm \"{}\"", row.name));
            }
            self.realms.insert(row.name, realm);
        }
        Some(())
    }
}

/// `RealmList::LoadAllowedClients`.
fn load_allowed_clients(server: &Server) -> Option<Vec<RealmBuildInfo>> {
    server.out(Level::Detail, "Loading allowed clients...");
    let rows = server.db.allowed_clients().ok()?;
    let mut builds = Vec::with_capacity(rows.len());
    for row in rows {
        let mut integrity_hash = [0u8; 20];
        if !row.integrity_hash.is_empty() {
            // C++ asserts 40 hex chars; a malformed value leaves zeros here.
            if let Some(bytes) = hex_to_bytes(&row.integrity_hash)
                && bytes.len() == 20
            {
                integrity_hash.copy_from_slice(&bytes);
            }
        }
        builds.push(RealmBuildInfo {
            major_version: row.major_version,
            minor_version: row.minor_version,
            bugfix_version: row.bugfix_version,
            _hotfix_version: row.hotfix_version.as_bytes().first().copied().unwrap_or(0),
            build: row.build,
            os: row.os,
            platform: row.platform,
            integrity_hash,
        });
    }
    Some(builds)
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Resolve a realm address like `DNS::ResolveDomainSingle(..., IPv4)`:
/// literal IPv4 or the first IPv4 DNS answer.
fn resolve_ipv4(name: &str) -> Option<Ipv4Addr> {
    if let Ok(ip) = name.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    (name, 0u16)
        .to_socket_addrs()
        .ok()?
        .find_map(|addr| match addr.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
}

/// Subnet mask string → CIDR, rejecting masks with holes
/// (`RealmList.cpp:222-236`).
fn subnet_mask_cidr(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from(mask);
    let inv = !bits;
    if inv & inv.wrapping_add(1) != 0 {
        return None;
    }
    Some(bits.count_ones() as u8)
}

/// One `UpdateRealms` row → `Realm` (the `UpdateRealm` body).
fn parse_realm_row(
    server: &Server,
    builds: &[RealmBuildInfo],
    row: &RealmlistRow,
) -> Option<Realm> {
    let mut flags = row.realm_flags;
    if flags & !REALM_FLAGS_ALLOWED_MASK != 0 {
        server.out(Level::Error, "Realm allowed have only OFFLINE Mask 0x2), or NEWPLAYERS (mask 0x20), or RECOMENDED (mask 0x40), or SPECIFICBUILD (mask 0x04) flags in DB");
        flags &= REALM_FLAGS_ALLOWED_MASK;
    }
    let Some(external_ip) = resolve_ipv4(&row.address) else {
        server.out(
            Level::Error,
            &format!(
                "Could not parse externalAddress: {} for realm \"{}\" (id {}) will skip realm update.",
                row.address, row.name, row.id
            ),
        );
        return None;
    };
    let Some(local_ip) = resolve_ipv4(&row.local_address) else {
        server.out(
            Level::Error,
            &format!(
                "Could not parse localAddress: {} for realm \"{}\" (id {}) will skip realm update.",
                row.local_address, row.name, row.id
            ),
        );
        return None;
    };
    let mask = row.local_subnet_mask.parse::<Ipv4Addr>().ok();
    let local_cidr = mask.and_then(subnet_mask_cidr);
    let Some(local_cidr) = local_cidr else {
        let what = if mask.is_none() {
            "Could not parse"
        } else {
            "Invalid"
        };
        server.out(
            Level::Error,
            &format!(
                "{} localSubnetMask: {} for realm \"{}\" (id {}) will skip realm update.",
                what, row.local_subnet_mask, row.name, row.id
            ),
        );
        return None;
    };

    // StrSplit(builds, " ") + atol per token, empty tokens included.
    let realm_builds: BTreeSet<u32> = row
        .realm_builds
        .split(' ')
        .map(|tok| tok.trim().parse::<i64>().unwrap_or(0) as u32)
        .collect();
    let first_build = realm_builds.iter().next().copied().unwrap_or(0) as u16;
    let mut build_info = RealmBuildInfo {
        major_version: 0,
        minor_version: 0,
        bugfix_version: 0,
        _hotfix_version: b' ',
        build: first_build,
        os: String::new(),
        platform: String::new(),
        integrity_hash: [0u8; 20],
    };
    if first_build != 0
        && let Some(found) = find_build_info(builds, first_build)
        && found.build == first_build
    {
        build_info = found.clone();
    }

    Some(Realm {
        id: row.id,
        icon: row.icon,
        flags,
        timezone: row.timezone,
        allowed_security_level: row.allowed_security_level.min(SEC_ADMINISTRATOR),
        population: row.population,
        builds: realm_builds,
        build_info,
        external_ip,
        local_ip,
        local_cidr,
        port: row.port as u16,
    })
}

/// Cached MD5 of a patch file (`ClientPatchCache`).
struct PatchEntry {
    modified: SystemTime,
    size: u64,
    md5: [u8; 16],
}

/// Shared server context (the C++ singletons, bundled).
struct Server {
    config: Config,
    log: Log,
    console_level: Level,
    db: Arc<LoginDb>,
    realms: Mutex<RealmState>,
    patch_cache: Mutex<HashMap<PathBuf, PatchEntry>>,
}

impl Server {
    /// `sLog.Out(LOG_BASIC, level, ...)`: file sink plus a stdout mirror
    /// (the C++ console sink; message text only at default `LogTime=0`).
    fn out(&self, level: Level, message: &str) {
        self.log.out(level, message);
        if level <= self.console_level {
            println!("{message}");
        }
    }

    fn patches_dir(&self) -> String {
        self.config.string_default("PatchesDir", "./patches")
    }

    /// `ClientPatchCache::LoadPatchesInfo`: hash every file in the dir.
    fn warm_patch_cache(&self) {
        let dir = self.patches_dir();
        self.out(
            Level::Debug,
            &format!("[PatchCache] Loading available game client patches from folder {dir}"),
        );
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                self.out(
                    Level::Debug,
                    &format!("[PatchCache] Calculate hash of {}", path.display()),
                );
                let _ = self.patch_md5(&path);
            }
        }
    }

    /// `ClientPatchCache::GetOrCalculateHash` keyed on (mtime, size).
    fn patch_md5(&self, path: &Path) -> Option<[u8; 16]> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        let size = meta.len();
        {
            let cache = self.patch_cache.lock().expect("patch cache poisoned");
            if let Some(entry) = cache.get(path)
                && entry.modified == modified
                && entry.size == size
            {
                return Some(entry.md5);
            }
        }
        let mut file = std::fs::File::open(path).ok()?;
        let mut hasher = md5_hasher();
        std::io::copy(&mut file, &mut hasher).ok()?;
        let md5 = hasher.finish();
        self.patch_cache
            .lock()
            .expect("patch cache poisoned")
            .insert(
                path.to_path_buf(),
                PatchEntry {
                    modified,
                    size,
                    md5,
                },
            );
        Some(md5)
    }
}

/// Tiny `Write` adapter so `std::io::copy` can feed the MD5 hasher.
struct Md5Writer(md5::Md5);

fn md5_hasher() -> Md5Writer {
    Md5Writer(<md5::Md5 as md5::Digest>::new())
}

impl Md5Writer {
    fn finish(self) -> [u8; 16] {
        <md5::Md5 as md5::Digest>::finalize(self.0).into()
    }
}

impl Write for Md5Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        <md5::Md5 as md5::Digest>::update(&mut self.0, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn level_from_config(value: i32) -> Level {
    match value {
        i32::MIN..=0 => Level::Error,
        1 => Level::Minimal,
        2 => Level::Basic,
        3 => Level::Detail,
        _ => Level::Debug,
    }
}

fn print_usage(exe: &str) {
    println!(
        "Usage: {exe} [<options>]\n    -v, --version               print version and exist\n    -c, --config <config_file>  use config_file as configuration file\n  Running as daemon functions:\n    -s run                      run as daemon\n    -s stop                     stop daemon"
    );
}

enum ArgOutcome {
    Run {
        config_path: String,
        stop_daemon: bool,
    },
    Exit(ExitCode),
}

/// `ParseServerStartupArguments` (POSIX flavour).
fn parse_args() -> ArgOutcome {
    let mut args = std::env::args();
    let exe = args.next().unwrap_or_else(|| "realmd".to_string());
    let mut config_path = String::new();
    let mut stop_daemon = false;
    while let Some(part) = args.next() {
        match part.as_str() {
            "-h" | "--help" => {
                print_usage(&exe);
                return ArgOutcome::Exit(ExitCode::SUCCESS);
            }
            "-v" | "--version" => {
                println!("Core revision: {}", mangos_build_info::full_version());
                return ArgOutcome::Exit(ExitCode::SUCCESS);
            }
            "-c" | "--config" => {
                let Some(next) = args.next() else {
                    println!("Error: Config filename required");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                };
                config_path = next;
            }
            "-s" => match args.next().as_deref() {
                Some("run") => {}
                Some("stop") => stop_daemon = true,
                Some(_) => {
                    println!("Error: Unknown service mode");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                }
                None => {
                    println!("Error: Config filename required");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                }
            },
            other => {
                println!("Error: Unknown argument provided '{other}'");
                print_usage(&exe);
                return ArgOutcome::Exit(ExitCode::FAILURE);
            }
        }
    }
    if config_path.is_empty() {
        config_path = DEFAULT_CONFIG_PATH.to_string();
    }
    ArgOutcome::Run {
        config_path,
        stop_daemon,
    }
}

/// `-s stop`: signal the PID recorded in the configured `PidFile`.
fn stop_via_pidfile(config: &Config) -> ExitCode {
    let pidfile = config.string_default("PidFile", "");
    if pidfile.is_empty() {
        return ExitCode::FAILURE;
    }
    let Ok(contents) = std::fs::read_to_string(&pidfile) else {
        return ExitCode::FAILURE;
    };
    let Some(pid) = contents.trim().parse::<u32>().ok() else {
        return ExitCode::FAILURE;
    };
    match std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Parse `host;port;user;password;db` (`LoginDatabaseInfo`) into a
/// mysql URL, applying the C++ "exactly 4 semicolons" format check.
fn login_db_url(dbstring: &str) -> Option<(String, String)> {
    if dbstring.matches(';').count() != 4 {
        return None;
    }
    let parts: Vec<&str> = dbstring.split(';').collect();
    let (host, port, user, password, db) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
    let masked = format!("{host};{port};{user};*;{db}");
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        percent_encode(user),
        percent_encode(password),
        host,
        port,
        percent_encode(db)
    );
    Some((url, masked))
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

// Mirrors the C++ main() startup sequence step for step.
#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let (config_path, stop_daemon) = match parse_args() {
        ArgOutcome::Run {
            config_path,
            stop_daemon,
        } => (config_path, stop_daemon),
        ArgOutcome::Exit(code) => return code,
    };

    let Ok(config) = Config::load_from_file(Path::new(&config_path)) else {
        println!("Could not find or parse configuration file {config_path}");
        return ExitCode::FAILURE;
    };

    if stop_daemon {
        return stop_via_pidfile(&config);
    }

    // sLog initialization (Log.cpp ctor): basic log file + levels.
    let logs_dir = config.string_default("LogsDir", "");
    let log = match Log::open(
        Path::new(&logs_dir),
        &config.string_default("LogFile.Basic", "Realmd.log"),
        config.bool_default("LogFile.TimeStamp", false),
        level_from_config(config.int_default("LogLevel.File", 2)),
    ) {
        Ok(log) => log,
        Err(err) => {
            println!("Could not open log file: {err}");
            return ExitCode::FAILURE;
        }
    };
    let console_level = level_from_config(config.int_default("LogLevel.Console", 2));

    let banner = |msg: &str| {
        log.out(Level::Minimal, msg);
        if Level::Minimal <= console_level {
            println!("{msg}");
        }
    };
    banner(&format!(
        "Core revision: {} [realm-daemon]",
        mangos_build_info::full_version()
    ));
    banner("<Ctrl-C> to stop.\n");
    banner(&format!("Using configuration file {config_path}."));

    if config.int_default("ConfVersion", 0) < REALMD_CONF_VERSION {
        for line in [
            "*****************************************************************************",
            " WARNING: Your realmd.conf version indicates your conf file is out of date!  ",
            "          Please check for updates, as your current default values may cause ",
            "          strange behavior.                                                  ",
            "*****************************************************************************",
        ] {
            log.out(Level::Error, line);
            println!("{line}");
        }
    }

    // StartDB()
    let dbstring = config.string_default("LoginDatabaseInfo", "");
    if dbstring.is_empty() {
        log.out(Level::Error, "Database not specified");
        return ExitCode::FAILURE;
    }
    let Some((db_url, masked)) = login_db_url(&dbstring) else {
        log.out(
            Level::Error,
            "Incorrectly formatted database connection string for logon database",
        );
        return ExitCode::FAILURE;
    };
    log.out(Level::Minimal, &format!("Database: {masked}"));
    if Level::Minimal <= console_level {
        println!("Database: {masked}");
    }
    let Ok(db) = LoginDb::spawn(&db_url) else {
        log.out(Level::Error, "Cannot connect to database");
        return ExitCode::FAILURE;
    };
    let db = Arc::new(db);

    // Migration gate (CheckRequiredMigrations).
    match db.applied_migrations() {
        Ok((applied, db_name)) => {
            let applied_set: BTreeSet<&str> = applied.iter().map(String::as_str).collect();
            let missing: Vec<&str> = REQUIRED_LOGON_MIGRATIONS
                .iter()
                .copied()
                .filter(|name| !applied_set.contains(name))
                .collect();
            if !missing.is_empty() {
                log.out(
                    Level::Minimal,
                    &format!("Database `{db_name}` is missing the following migrations:"),
                );
                for name in &missing {
                    log.out(Level::Minimal, &format!("\t{name}"));
                }
                return ExitCode::FAILURE;
            }
            let required: BTreeSet<&str> = REQUIRED_LOGON_MIGRATIONS.iter().copied().collect();
            let extra: Vec<&str> = applied_set.difference(&required).copied().collect();
            if !extra.is_empty() {
                log.out(
                    Level::Minimal,
                    &format!("WARNING! Database `{db_name}` has the following extra migrations:"),
                );
                for name in &extra {
                    log.out(Level::Minimal, &format!("\t{name}"));
                }
            }
        }
        Err(_) => return ExitCode::FAILURE,
    }

    // PID file.
    let pidfile = config.string_default("PidFile", "");
    if !pidfile.is_empty() {
        let pid = std::process::id();
        if std::fs::write(&pidfile, format!("{pid}\n")).is_err() {
            log.out(
                Level::Error,
                &format!("Cannot create PID file {pidfile}.\n"),
            );
            return ExitCode::FAILURE;
        }
        log.out(Level::Basic, &format!("Daemon PID: {pid}\n"));
    }

    if config.bool_default("GeoLocking", false) && !matches!(db.geoip_table_nonempty(), Ok(true)) {
        log.out(
            Level::Error,
            "The geoip table cannot be empty when geolocking is enabled.",
        );
        return ExitCode::FAILURE;
    }

    // Realm list + allowed builds (needs Server for logging; realms
    // start empty and are filled through the same initialize path).
    let update_interval = config.int_default("RealmsStateUpdateDelay", 20).max(0) as u64;
    let server = Arc::new(Server {
        config,
        log,
        console_level,
        db,
        realms: Mutex::new(RealmState {
            realms: BTreeMap::new(),
            builds: Vec::new(),
            update_interval_secs: update_interval,
            next_update: Instant::now(),
        }),
        patch_cache: Mutex::new(HashMap::new()),
    });

    let Some(state) = RealmState::initialize(&server, update_interval) else {
        return ExitCode::FAILURE;
    };
    if state.realms.is_empty() {
        server.out(Level::Error, "No valid realms specified.");
        return ExitCode::FAILURE;
    }
    if state.builds.is_empty() {
        server.out(Level::Error, "No valid client builds specified.");
        return ExitCode::FAILURE;
    }
    *server.realms.lock().expect("realm state poisoned") = state;

    server.warm_patch_cache();
    let _ = server.db.cleanup_expired_bans();

    let bind_ip = server.config.string_default("BindIP", "0.0.0.0");
    let bind_port = server
        .config
        .int_default("RealmServerPort", DEFAULT_REALMSERVER_PORT) as u16;
    let Ok(listener) = TcpListener::bind((bind_ip.as_str(), bind_port)) else {
        server.out(
            Level::Error,
            &format!(
                "MaNGOS realmd can not bind to {bind_ip}:{bind_port}  -  Is the port already in use?"
            ),
        );
        return ExitCode::FAILURE;
    };

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let server = Arc::clone(&server);
                std::thread::spawn(move || handle_connection(&server, stream));
            }
            Err(err) => {
                server.out(Level::Error, &format!("accept failed: {err}"));
            }
        }
    }
    ExitCode::SUCCESS
}

/// `AuthSocket::Status` (the handler-table gate).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Challenge,
    LogonProof,
    ReconProof,
    Authed,
    Patch,
    Invalid,
}

/// Per-connection state (`AuthSocket` members the flow uses).
struct Session {
    status: Status,
    build: u16,
    login: String,
    os: String,
    platform: String,
    localization_name: String,
    account_id: u32,
    srp: Option<SrpSession>,
    strong_key: Option<BigUint>,
    last_ip: String,
    lock_flags: u8,
    security_info: String,
    geo_unlock_pin: u32,
    prompt_pin: bool,
    grid_seed: u32,
    server_security_salt: BigUint,
    reconnect_proof: BigUint,
    default_security: u8,
    security_on_realm: HashMap<u32, u8>,
    last_realm_list_request: Option<Instant>,
    pending_patch: Option<PathBuf>,
}

impl Session {
    fn new() -> Session {
        Session {
            status: Status::Challenge,
            build: 0,
            login: String::new(),
            os: String::new(),
            platform: String::new(),
            localization_name: String::new(),
            account_id: 0,
            srp: None,
            strong_key: None,
            last_ip: String::new(),
            lock_flags: 0,
            security_info: String::new(),
            geo_unlock_pin: 0,
            prompt_pin: false,
            grid_seed: 0,
            server_security_salt: BigUint::ZERO,
            reconnect_proof: BigUint::ZERO,
            default_security: 0,
            security_on_realm: HashMap::new(),
            last_realm_list_request: None,
            pending_patch: None,
        }
    }

    /// `GetSecurityOn` (`AuthSocket.cpp:92-98`).
    fn security_on(&self, realm_id: u32) -> u8 {
        self.security_on_realm
            .get(&realm_id)
            .copied()
            .unwrap_or(self.default_security)
    }

    /// `VerifyVersion` (`AuthSocket.cpp:1390-1428`).
    fn verify_version(
        &self,
        server: &Server,
        builds: &[RealmBuildInfo],
        a: &[u8],
        version_proof: &[u8; 20],
        is_reconnect: bool,
    ) -> bool {
        let allowed = find_build_info_full(builds, self.build, &self.os, &self.platform);
        if allowed.is_empty() {
            return false;
        }
        if !server.config.bool_default("StrictVersionCheck", false) {
            return true;
        }
        let zeros = [0u8; 20];
        for info in allowed {
            let version_hash = if is_reconnect {
                &zeros
            } else if info.integrity_hash == zeros {
                return true; // not filled serverside
            } else {
                &info.integrity_hash
            };
            let mut hasher = Sha1::new();
            hasher.update(a);
            hasher.update(version_hash);
            if hasher.finalize().as_slice() == version_proof.as_slice() {
                return true;
            }
        }
        false
    }
}

/// Flow control for one handler invocation.
enum Flow {
    Continue,
    Close,
}

/// A random `BigNumber` of `bytes` bytes like `SetRand(bytes*8)`
/// (top and bottom bits set), returned with its padded-LE wire form.
fn random_biguint(bytes: usize) -> BigUint {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    if let Some(first) = buf.first_mut() {
        *first |= 0x80;
    }
    if let Some(last) = buf.last_mut() {
        *last |= 0x01;
    }
    BigUint::from_bytes_be(&buf)
}

fn random_u32() -> u32 {
    let mut buf = [0u8; 4];
    rand::fill(&mut buf);
    u32::from_le_bytes(buf)
}

/// Connection state bundled for the handler methods.
struct Conn {
    stream: TcpStream,
    remote_ip_string: String,
    remote_ip: IpAddr,
    deadline: Option<Instant>,
    session: Session,
}

impl Conn {
    /// `read_exact` bounded by the `MaxSessionDuration` deadline.
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        if let Some(deadline) = self.deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "MaxSessionDuration reached",
                ));
            }
            self.stream.set_read_timeout(Some(remaining))?;
        }
        self.stream.read_exact(buf)
    }

    fn read_vec(&mut self, len: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(data)
    }
}

/// `AuthSocket::Start` + `DoRecvIncomingData` dispatch loop.
fn handle_connection(server: &Server, stream: TcpStream) {
    let remote = stream
        .peer_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let _ = stream.set_nodelay(true);
    let max_duration = server.config.int_default("MaxSessionDuration", 300);
    let deadline =
        (max_duration > 0).then(|| Instant::now() + Duration::from_secs(max_duration as u64));
    let mut conn = Conn {
        stream,
        remote_ip_string: remote.ip().to_string(),
        remote_ip: remote.ip(),
        deadline,
        session: Session::new(),
    };

    loop {
        let mut cmd_byte = [0u8; 1];
        if conn.read_exact(&mut cmd_byte).is_err() {
            break;
        }
        let flow = match (cmd_byte[0], conn.session.status) {
            (cmd::AUTH_LOGON_CHALLENGE, Status::Challenge) => {
                handle_logon_challenge(server, &mut conn, false)
            }
            (cmd::AUTH_LOGON_PROOF, Status::LogonProof) => handle_logon_proof(server, &mut conn),
            (cmd::AUTH_RECONNECT_CHALLENGE, Status::Challenge) => {
                handle_logon_challenge(server, &mut conn, true)
            }
            (cmd::AUTH_RECONNECT_PROOF, Status::ReconProof) => {
                handle_reconnect_proof(server, &mut conn)
            }
            (cmd::REALM_LIST, Status::Authed) => handle_realm_list(server, &mut conn),
            (cmd::XFER_ACCEPT, Status::Patch) => handle_xfer_accept(server, &mut conn),
            (cmd::XFER_RESUME, Status::Patch) => handle_xfer_resume(server, &mut conn),
            (cmd::XFER_CANCEL, Status::Patch) => Flow::Close,
            (known, _) if is_known_cmd(known) => {
                server.out(
                    Level::Error,
                    &format!(
                        "[{}] Received unauthorized command {}",
                        conn.remote_ip_string, known
                    ),
                );
                Flow::Close
            }
            (unknown, _) => {
                server.out(
                    Level::Error,
                    &format!("[Auth] got unknown packet cmd {unknown}"),
                );
                Flow::Close
            }
        };
        match flow {
            Flow::Continue => {}
            Flow::Close => break,
        }
    }
    server.out(
        Level::Basic,
        &format!("[{}] Connection closed", conn.remote_ip_string),
    );
}

fn is_known_cmd(byte: u8) -> bool {
    matches!(
        byte,
        cmd::AUTH_LOGON_CHALLENGE
            | cmd::AUTH_LOGON_PROOF
            | cmd::AUTH_RECONNECT_CHALLENGE
            | cmd::AUTH_RECONNECT_PROOF
            | cmd::REALM_LIST
            | cmd::XFER_ACCEPT
            | cmd::XFER_RESUME
            | cmd::XFER_CANCEL
    )
}

/// Parse + stash the shared challenge-body fields (both challenge
/// handlers repeat this dance in C++).
fn read_challenge_body(conn: &mut Conn) -> Option<LogonChallengeBody> {
    let header_bytes = conn.read_vec(3).ok()?;
    let mut reader = PacketReader::new(&header_bytes);
    let header = parse_logon_challenge_header(&mut reader).ok()?;
    let body_len = usize::from(header.size);
    if body_len < MIN_CHALLENGE_BODY {
        return None;
    }
    let body_bytes = conn.read_vec(body_len).ok()?;
    let username_len = usize::from(*body_bytes.get(29)?);
    if username_len > MAX_USERNAME_LEN {
        return None;
    }
    // The C++ reads the body into a fixed struct: bytes past the
    // struct are unread there (they cannot occur from a real client).
    let body = parse_logon_challenge_body(&body_bytes).ok()?;

    conn.session.build = body.build;
    conn.session.os = reversed_cstr3(body.os);
    conn.session.platform = reversed_cstr3(body.platform);
    let mut country = body.country;
    country.reverse();
    conn.session.localization_name = String::from_utf8_lossy(&country).into_owned();
    conn.session.login.clone_from(&body.username);
    Some(body)
}

/// C++: `tag[3] = 0; str = (char*)tag; reverse(str)`.
fn reversed_cstr3(tag: [u8; 4]) -> String {
    let mut bytes: Vec<u8> = tag[..3].iter().copied().take_while(|&b| b != 0).collect();
    bytes.reverse();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `_HandleLogonChallenge` / `_HandleReconnectChallenge`
/// (`AuthSocket.cpp:198-466`, `774-880`).
#[allow(clippy::too_many_lines)]
fn handle_logon_challenge(server: &Server, conn: &mut Conn, reconnect: bool) -> Flow {
    conn.session.status = Status::Invalid;
    if read_challenge_body(conn).is_none() {
        return Flow::Close;
    }
    let login = conn.session.login.clone();
    let remote_ip = conn.remote_ip_string.clone();

    if reconnect {
        return match server.db.sessionkey_by_username(login.clone()) {
            Ok(Some((key_hex, account_id))) => {
                conn.session.strong_key = bn_from_hex(&key_hex);
                conn.session.account_id = account_id;
                conn.session.reconnect_proof = random_biguint(16);
                let proof_bytes = bn_as_byte_array_padded(&conn.session.reconnect_proof, 16);
                let mut pkt = PacketWriter::new();
                pkt.write_u8(cmd::AUTH_RECONNECT_CHALLENGE)
                    .write_u8(0)
                    .write_bytes(&proof_bytes)
                    .write_bytes(&VERSION_CHALLENGE);
                if conn.write_all(pkt.as_slice()).is_err() {
                    return Flow::Close;
                }
                conn.session.status = Status::ReconProof;
                Flow::Continue
            }
            Ok(None) => {
                server.out(
                    Level::Error,
                    &format!(
                        "user {login} tried to login and we cannot find his session key in the database."
                    ),
                );
                Flow::Close
            }
            Err(_) => Flow::Close,
        };
    }

    // IP ban.
    match server.db.ip_ban_unbandate(remote_ip.clone()) {
        Ok(Some(_)) => {
            server.out(
                Level::Basic,
                &format!(
                    "[AuthChallenge] Banned ip '{remote_ip}' tries to login with account '{login}'!"
                ),
            );
            let _ = conn.write_all(&build_challenge_failure(result::WOW_FAIL_FAIL_NOACCESS));
            return Flow::Continue;
        }
        Ok(None) => {}
        Err(_) => return Flow::Close,
    }

    let account = match server.db.account_by_username(login.clone()) {
        Ok(Some(account)) => account,
        Ok(None) => {
            let _ = conn.write_all(&build_challenge_failure(result::WOW_FAIL_UNKNOWN_ACCOUNT));
            return Flow::Continue;
        }
        Err(_) => return Flow::Close,
    };

    // Email-verification gate.
    let mut require_verification = server.config.bool_default("ReqEmailVerification", false);
    let require_since = server.config.int_default("ReqEmailSince", 0);
    if require_since > 0 {
        require_verification =
            require_verification && account.joindate_unix >= require_since as u32;
    }
    if require_verification && account.email_verif == 0 {
        server.out(
            Level::Basic,
            &format!(
                "[AuthChallenge] Account '{login}' using IP '{remote_ip} 'email address requires email verification - rejecting login"
            ),
        );
        let _ = conn.write_all(&build_challenge_failure(result::WOW_FAIL_UNKNOWN_ACCOUNT));
        return Flow::Continue;
    }

    conn.session.lock_flags = account.locked;
    conn.session.security_info.clone_from(&account.security);
    conn.session.last_ip.clone_from(&account.last_ip);
    conn.session.geo_unlock_pin = account.geolock_pin;

    // IP lock (the C++ keeps building the packet and gates below).
    let mut ip_reject: Option<u8> = None;
    let mut locked = false;
    if account.locked & LOCK_IP_LOCK != 0 && account.last_ip != remote_ip {
        if account.locked & LOCK_TOTP != LOCK_TOTP
            && account.locked & LOCK_FIXED_PIN != LOCK_FIXED_PIN
        {
            ip_reject = Some(result::WOW_FAIL_SUSPENDED);
        }
        locked = true;
    }

    let verifier = bn_from_hex(&account.v).filter(|v| *v != BigUint::ZERO);
    let salt = bn_from_hex(&account.s).filter(|s| *s != BigUint::ZERO);
    let (Some(verifier), Some(salt)) = (verifier, salt) else {
        server.out(
            Level::Error,
            &format!("[AuthChallenge] Broken v/s values in database for account {login}!"),
        );
        let _ = conn.write_all(&build_challenge_failure(result::WOW_FAIL_FAIL_NOACCESS));
        return Flow::Continue;
    };

    // Locked without a PIN factor: the C++ writes only the SUSPENDED
    // reply assembled above and stops.
    if locked && account.locked & (LOCK_FIXED_PIN | LOCK_TOTP) == 0 {
        if let Some(error) = ip_reject {
            let _ = conn.write_all(&build_challenge_failure(error));
        }
        return Flow::Continue;
    }

    // Account ban.
    match server.db.active_account_ban(account.id) {
        Ok(Some((bandate, unbandate))) => {
            let (error, what) = if bandate == unbandate {
                (result::WOW_FAIL_BANNED, "Banned")
            } else {
                (result::WOW_FAIL_SUSPENDED, "Temporarily banned")
            };
            server.out(
                Level::Basic,
                &format!(
                    "[AuthChallenge] {what} account '{login}' using IP '{remote_ip}' tries to login!"
                ),
            );
            let _ = conn.write_all(&build_challenge_failure(error));
            return Flow::Continue;
        }
        Ok(None) => {}
        Err(_) => return Flow::Close,
    }

    // SRP host ephemeral + challenge response.
    let srp = SrpSession::host_ephemeral(verifier, salt);
    let mut prompt_pin = locked;
    if (!locked && account.locked & LOCK_ALWAYS_ENFORCE == LOCK_ALWAYS_ENFORCE)
        || account.geolock_pin != 0
    {
        prompt_pin = true;
    }
    conn.session.prompt_pin = prompt_pin;
    let pin = if prompt_pin {
        server.out(
            Level::Basic,
            &format!(
                "[AuthChallenge] Account '{login}' using IP '{remote_ip}' requires PIN authentication"
            ),
        );
        conn.session.grid_seed = random_u32();
        conn.session.server_security_salt = random_biguint(16);
        let salt_bytes = bn_as_byte_array_padded(&conn.session.server_security_salt, 16);
        let mut salt16 = [0u8; 16];
        salt16.copy_from_slice(&salt_bytes[..16]);
        Some(PinGrid {
            grid_seed_be: conn.session.grid_seed.swap_bytes(),
            salt: salt16,
        })
    } else {
        None
    };
    let g_bytes = bn_as_byte_array(&BigUint::from(G));
    let n_bytes = bn_as_byte_array_padded(&prime_n(), 32);
    let pkt = build_logon_challenge_success(
        &srp.public_b_bytes32(),
        &g_bytes,
        &n_bytes,
        &srp.salt_bytes(),
        conn.session.build,
        pin,
    );
    conn.session.srp = Some(srp);

    // LoadAccountSecurityLevels.
    if let Ok(rows) = server.db.account_access(account.id) {
        for (gmlevel, realm_id) in rows {
            if realm_id < 0 {
                conn.session.default_security = gmlevel;
            } else {
                conn.session
                    .security_on_realm
                    .insert(realm_id as u32, gmlevel);
            }
        }
    }
    conn.session.account_id = account.id;
    if conn.write_all(&pkt).is_err() {
        return Flow::Close;
    }
    conn.session.status = Status::LogonProof;
    Flow::Continue
}

/// `_HandleLogonProof` + `__PostRecv` (`AuthSocket.cpp:469-771`).
#[allow(clippy::too_many_lines)]
fn handle_logon_proof(server: &Server, conn: &mut Conn) -> Flow {
    conn.session.status = Status::Invalid;
    // sizeof(sAuthLogonProof_C_Pre_1_11_0)=73, +securityFlags=74.
    let proof_len = if conn.session.build < 5428 { 73 } else { 74 };
    let Ok(mut data) = conn.read_vec(proof_len) else {
        return Flow::Close;
    };
    // PIN block follows when announced; unsupported flag combos close.
    if conn.session.build >= 5428 {
        let flags = data[73];
        if flags != 0 {
            if flags & 0x01 == 0 {
                return Flow::Close;
            }
            let Ok(pin_block) = conn.read_vec(36) else {
                return Flow::Close;
            };
            data.extend_from_slice(&pin_block);
        }
    }
    let Ok(proof) = parse_logon_proof(&data, conn.session.build) else {
        return Flow::Close;
    };

    // Version gate → patch offer.
    let (valid_version, builds_snapshot) = {
        let state = server.realms.lock().expect("realm state poisoned");
        (
            find_build_info(&state.builds, conn.session.build).is_some(),
            state.builds.clone(),
        )
    };
    if !valid_version {
        return handle_invalid_version(server, conn);
    }

    let Some(mut srp) = conn.session.srp.take() else {
        return Flow::Close;
    };
    if !srp.calculate_session_key(&proof.a) {
        server.out(
            Level::Basic,
            &format!(
                "[AuthChallenge] Session calculation failed for account {}!",
                conn.session.login
            ),
        );
        return Flow::Close;
    }
    srp.calculate_proof(&conn.session.login.clone());

    // PIN result.
    let mut pin_result = true;
    if conn.session.prompt_pin && proof.security_flags == 0 {
        pin_result = false;
    }
    if conn.session.prompt_pin && proof.security_flags != 0 {
        let pin_data = (
            proof.pin_salt.unwrap_or([0; 16]),
            proof.pin_hash.unwrap_or([0; 20]),
        );
        if conn.session.lock_flags & LOCK_FIXED_PIN == LOCK_FIXED_PIN {
            let pin = conn.session.security_info.parse::<u32>().unwrap_or(0);
            pin_result = verify_pin_data(conn, pin, pin_data);
        } else if conn.session.lock_flags & LOCK_TOTP == LOCK_TOTP {
            pin_result = false;
            for interval in -2..2 {
                let Some(pin) = generate_totp_pin(&conn.session.security_info, interval) else {
                    break;
                };
                if verify_pin_data(conn, pin, pin_data) {
                    pin_result = true;
                    break;
                }
            }
        } else if conn.session.geo_unlock_pin != 0 {
            let pin = conn.session.geo_unlock_pin;
            pin_result = verify_pin_data(conn, pin, pin_data);
        } else {
            pin_result = false;
            server.out(
                Level::Error,
                &format!(
                    "Invalid PIN flags set for user {} - user cannot log-in until fixed",
                    conn.session.login
                ),
            );
        }
    }

    // Observed C++ semantics: `!srp.Proof(M1) && pinResult` with the
    // inverted Proof == M1 matches (see mangos-protocol docs).
    if !(srp.proof_matches(&proof.m1) && pin_result) {
        return wrong_password(server, conn);
    }

    if !conn
        .session
        .verify_version(server, &builds_snapshot, &proof.a, &proof.crc_hash, false)
    {
        server.out(
            Level::Basic,
            &format!(
                "[AuthChallenge] Account {} tried to login with modified client!",
                conn.session.login
            ),
        );
        let _ = conn.write_all(&build_logon_failure(
            cmd::AUTH_LOGON_PROOF,
            result::WOW_FAIL_VERSION_INVALID,
        ));
        return Flow::Continue;
    }

    // Geolock enforcement (geoip lookups + pin writes + mail) is out of
    // Phase 1 scope; with GeoLocking off (production default read) the
    // C++ takes exactly this path.
    server.out(
        Level::Basic,
        &format!(
            "[AuthChallenge] Account '{}' using IP '{}' successfully authenticated",
            conn.session.login, conn.remote_ip_string
        ),
    );

    let update = server.db.update_session_on_proof(
        conn.session.login.clone(),
        srp.session_key_hex(),
        conn.remote_ip_string.clone(),
        locale_by_name(&conn.session.localization_name),
        conn.session.os.clone(),
        conn.session.platform.clone(),
    );
    if update.is_err() {
        server.out(
            Level::Error,
            &format!(
                "Unable to update login stats for account '{}'",
                conn.session.login
            ),
        );
    }

    let m2 = srp.finalize();
    conn.session.srp = Some(srp);
    let pkt = build_logon_proof_success(conn.session.build, &m2);
    if conn.write_all(&pkt).is_err() {
        return Flow::Close;
    }
    conn.session.status = Status::Authed;
    Flow::Continue
}

/// The wrong-password tail of `__PostRecv` (autoban + reply).
fn wrong_password(server: &Server, conn: &mut Conn) -> Flow {
    server.out(
        Level::Basic,
        &format!(
            "[AuthChallenge] Account '{}' using IP '{}' tried to login with wrong password!",
            conn.session.login, conn.remote_ip_string
        ),
    );
    let max_wrong = server.config.int_default("WrongPass.MaxCount", 0);
    if max_wrong > 0
        && let Ok(Some((account_id, failed_logins))) =
            server.db.record_failed_login(conn.session.login.clone())
        && failed_logins >= max_wrong as u32
    {
        let ban_time = server.config.int_default("WrongPass.BanTime", 600) as u32;
        if server.config.bool_default("WrongPass.BanType", false) {
            let _ = server.db.insert_account_autoban(account_id, ban_time);
            server.out(
                Level::Basic,
                &format!(
                    "[AuthChallenge] Account '{}' using  IP '{}' got banned for '{}' seconds because it failed to authenticate '{}' times",
                    conn.session.login, conn.remote_ip_string, ban_time, failed_logins
                ),
            );
        } else {
            let _ = server
                .db
                .insert_ip_autoban(conn.remote_ip_string.clone(), ban_time);
            server.out(
                Level::Basic,
                &format!(
                    "[AuthChallenge] IP '{}' got banned for '{}' seconds because account '{}' failed to authenticate '{}' times",
                    conn.remote_ip_string, ban_time, conn.session.login, failed_logins
                ),
            );
        }
    }
    let mut pkt = build_logon_failure(cmd::AUTH_LOGON_PROOF, result::WOW_FAIL_UNKNOWN_ACCOUNT);
    if conn.session.build > 6005 {
        pkt.extend_from_slice(&[0, 0]);
    }
    let _ = conn.write_all(&pkt);
    Flow::Continue
}

/// `_HandleLogonProof__PostRecv_HandleInvalidVersion`
/// (`AuthSocket.cpp:511-575`): offer the build's patch MPQ if present.
fn handle_invalid_version(server: &Server, conn: &mut Conn) -> Flow {
    if conn.session.pending_patch.is_some() {
        server.out(
            Level::Basic,
            "_HandleLogonProof__PostRecv m_patch is already set?? The client should accept the XFER!",
        );
        return Flow::Close;
    }
    let file_name = format!(
        "{}{}.mpq",
        conn.session.build, conn.session.localization_name
    );
    let path = PathBuf::from(server.patches_dir()).join(&file_name);
    if !path.is_file() {
        server.out(
            Level::Debug,
            &format!(
                "[AuthChallenge] {} is not a valid client version!",
                conn.session.build
            ),
        );
        server.out(
            Level::Debug,
            &format!("[AuthChallenge] Patch {} not found", path.display()),
        );
        let _ = conn.write_all(&build_challenge_failure(result::WOW_FAIL_VERSION_INVALID));
        return Flow::Continue;
    }
    let (Some(md5), Ok(meta)) = (server.patch_md5(&path), std::fs::metadata(&path)) else {
        return Flow::Close;
    };
    conn.session.pending_patch = Some(path);

    let mut pkt = PacketWriter::new();
    pkt.write_u8(cmd::AUTH_LOGON_PROOF)
        .write_u8(result::WOW_FAIL_VERSION_UPDATE)
        .write_u8(cmd::XFER_INITIATE)
        .write_u8(5)
        .write_bytes(b"Patch")
        .write_u64_le(meta.len())
        .write_bytes(&md5);
    if conn.write_all(pkt.as_slice()).is_err() {
        return Flow::Close;
    }
    conn.session.status = Status::Patch;
    Flow::Continue
}

/// `_HandleReconnectProof` (`AuthSocket.cpp:883-940`).
fn handle_reconnect_proof(server: &Server, conn: &mut Conn) -> Flow {
    conn.session.status = Status::Invalid;
    let Ok(data) = conn.read_vec(57) else {
        return Flow::Close;
    };
    let Ok(proof) = parse_reconnect_proof(&data) else {
        return Flow::Close;
    };
    let (Some(strong_key), false) = (
        conn.session.strong_key.clone(),
        conn.session.login.is_empty(),
    ) else {
        return Flow::Close;
    };
    if conn.session.reconnect_proof == BigUint::ZERO || strong_key == BigUint::ZERO {
        return Flow::Close;
    }
    if !verify_reconnect_proof(
        &conn.session.login,
        &proof.r1,
        &conn.session.reconnect_proof,
        &strong_key,
        &proof.r2,
    ) {
        server.out(
            Level::Error,
            &format!(
                "user {} tried to login, but session invalid.",
                conn.session.login
            ),
        );
        return Flow::Close;
    }
    let builds_snapshot = {
        let state = server.realms.lock().expect("realm state poisoned");
        state.builds.clone()
    };
    if !conn
        .session
        .verify_version(server, &builds_snapshot, &proof.r1, &proof.r3, true)
    {
        // C++ builds this packet but never sends it (AuthSocket.cpp:918-922).
        return Flow::Close;
    }
    let pkt = build_logon_failure(cmd::AUTH_RECONNECT_PROOF, result::WOW_SUCCESS);
    if conn.write_all(&pkt).is_err() {
        return Flow::Close;
    }
    conn.session.status = Status::Authed;
    Flow::Continue
}

/// `_HandleRealmList` + `LoadRealmlistAndWriteIntoBuffer`
/// (`AuthSocket.cpp:942-1107`).
fn handle_realm_list(server: &Server, conn: &mut Conn) -> Flow {
    let mut skip = [0u8; 4];
    if conn.read_exact(&mut skip).is_err() {
        return Flow::Close;
    }
    let min_delay = server.config.int_default("MinRealmListDelay", 1);
    let now = Instant::now();
    if let Some(last) = conn.session.last_realm_list_request {
        let delay = now.duration_since(last).as_secs() as i32;
        if delay < min_delay {
            server.out(
                Level::Error,
                &format!(
                    "user {} IP {} is sending CMD_REALM_LIST too frequently. Delay = {} seconds",
                    conn.session.login, conn.remote_ip_string, delay
                ),
            );
            return Flow::Close;
        }
    }
    conn.session.last_realm_list_request = Some(now);

    let (realms, builds) = {
        let mut state = server.realms.lock().expect("realm state poisoned");
        state.update_if_need(server);
        (state.realms.clone(), state.builds.clone())
    };

    let mut body = PacketWriter::new();
    if conn.session.build < 6299 {
        body.write_u32_le(0).write_u8(realms.len() as u8);
        for (name, realm) in &realms {
            write_realm_row_v1(server, conn, &builds, &mut body, name, realm);
        }
        body.write_u16_le(0x0002);
    } else {
        body.write_u32_le(0).write_u16_le(realms.len() as u16);
        for (name, realm) in &realms {
            write_realm_row_v2(server, conn, &builds, &mut body, name, realm);
        }
        body.write_u16_le(0x0010);
    }

    let mut pkt = PacketWriter::new();
    pkt.write_u8(cmd::REALM_LIST)
        .write_u16_le(body.as_slice().len() as u16)
        .write_bytes(body.as_slice());
    if conn.write_all(pkt.as_slice()).is_err() {
        return Flow::Close;
    }
    Flow::Continue
}

fn realm_num_chars(server: &Server, conn: &Conn, realm_id: u32) -> u8 {
    server
        .db
        .num_chars(realm_id, conn.session.account_id)
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// Pre-2.0.3 realm row (`AuthSocket.cpp:997-1043`).
fn write_realm_row_v1(
    server: &Server,
    conn: &Conn,
    builds: &[RealmBuildInfo],
    out: &mut PacketWriter,
    name: &str,
    realm: &Realm,
) {
    let num_chars = realm_num_chars(server, conn, realm.id);
    let ok_build = realm.builds.contains(&u32::from(conn.session.build));
    let build_info = if ok_build {
        find_build_info(builds, conn.session.build).unwrap_or(&realm.build_info)
    } else {
        &realm.build_info
    };
    let mut flags = realm.flags;
    let mut display_name = name.to_string();
    if flags & REALM_FLAG_SPECIFYBUILD != 0 {
        let _ = write!(
            display_name,
            " ({},{},{})",
            build_info.major_version, build_info.minor_version, build_info.bugfix_version
        );
    }
    if !ok_build || realm.allowed_security_level > conn.session.security_on(realm.id) {
        flags |= REALM_FLAG_OFFLINE;
    }
    out.write_u32_le(u32::from(realm.icon))
        .write_u8(flags)
        .write_cstring(&display_name)
        .write_cstring(&realm.address_for_client(conn.remote_ip))
        .write_f32_le(realm.population)
        .write_u8(num_chars)
        .write_u8(realm_category_id(
            builds,
            conn.session.build,
            realm.timezone,
        ))
        .write_u8(0);
}

/// 2.0.3+ realm row (`AuthSocket.cpp:1052-1103`).
fn write_realm_row_v2(
    server: &Server,
    conn: &Conn,
    builds: &[RealmBuildInfo],
    out: &mut PacketWriter,
    name: &str,
    realm: &Realm,
) {
    let num_chars = realm_num_chars(server, conn, realm.id);
    let ok_build = realm.builds.contains(&u32::from(conn.session.build));
    let build_info = if ok_build {
        find_build_info(builds, conn.session.build).unwrap_or(&realm.build_info)
    } else {
        &realm.build_info
    };
    let lock = u8::from(realm.allowed_security_level > conn.session.security_on(realm.id));
    let mut flags = realm.flags;
    if !ok_build {
        flags |= REALM_FLAG_OFFLINE;
    }
    out.write_u8(realm.icon)
        .write_u8(lock)
        .write_u8(flags)
        .write_cstring(name)
        .write_cstring(&realm.address_for_client(conn.remote_ip))
        .write_f32_le(realm.population)
        .write_u8(num_chars)
        .write_u8(realm_category_id(
            builds,
            conn.session.build,
            realm.timezone,
        ))
        .write_u8(realm.id as u8);
    if flags & REALM_FLAG_SPECIFYBUILD != 0 {
        out.write_u8(build_info.major_version)
            .write_u8(build_info.minor_version)
            .write_u8(build_info.bugfix_version)
            .write_u16_le(conn.session.build);
    }
}

/// `_HandleXferAccept` + `RepeatInternalXferLoop`.
fn handle_xfer_accept(server: &Server, conn: &mut Conn) -> Flow {
    let Some(path) = conn.session.pending_patch.clone() else {
        server.out(
            Level::Basic,
            &format!(
                "User '{}' tried to get patch file, but there is no patch file defined?",
                conn.session.login
            ),
        );
        return Flow::Close;
    };
    stream_patch(conn, &path, 0)
}

/// `_HandleXferResume`.
fn handle_xfer_resume(server: &Server, conn: &mut Conn) -> Flow {
    let Some(path) = conn.session.pending_patch.clone() else {
        server.out(
            Level::Basic,
            &format!(
                "User '{}' tried to get patch file, but there is no patch file defined?",
                conn.session.login
            ),
        );
        return Flow::Close;
    };
    let Ok(bytes) = conn.read_vec(8) else {
        return Flow::Close;
    };
    let start = i64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"));
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    if start < 0 || start as u64 >= size {
        server.out(
            Level::Basic,
            &format!(
                "[XFER] User '{}' tried to resume download outside file bounds",
                conn.session.login
            ),
        );
        return Flow::Close;
    }
    stream_patch(conn, &path, start as u64)
}

/// Send `XFER_DATA_CHUNK`s (cmd, u16 len, ≤4096 bytes) until EOF.
fn stream_patch(conn: &mut Conn, path: &Path, offset: u64) -> Flow {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Flow::Close;
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Flow::Close;
    }
    let mut data = [0u8; 4096];
    loop {
        let read = match file.read(&mut data) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Flow::Close,
        };
        let mut chunk = Vec::with_capacity(3 + read);
        chunk.push(cmd::XFER_DATA);
        chunk.extend_from_slice(&(read as u16).to_le_bytes());
        chunk.extend_from_slice(&data[..read]);
        if conn.write_all(&chunk).is_err() {
            return Flow::Close;
        }
    }
    // C++ ends the loop here without re-arming a read.
    Flow::Close
}

/// `VerifyPinData` (`AuthSocket.cpp:1162-1232`):
/// `H(client_salt | BN(H(server_salt | ascii(grid-remapped pin))))`
/// compared as `BigNumber`s.
fn verify_pin_data(conn: &Conn, pin: u32, pin_data: ([u8; 16], [u8; 20])) -> bool {
    let (client_salt, client_hash) = pin_data;
    let mut grid: Vec<u8> = (0..10).collect();
    let mut remapped: Vec<u8> = Vec::with_capacity(10);
    let mut seed = u64::from(conn.session.grid_seed);
    for i in (1..=10usize).rev() {
        let remainder = (seed % i as u64) as usize;
        seed /= i as u64;
        remapped.push(grid[remainder]);
        grid.remove(remainder);
    }

    let mut pin_bytes: Vec<u8> = Vec::new();
    let mut rest = pin;
    while rest != 0 {
        pin_bytes.push((rest % 10) as u8);
        rest /= 10;
    }
    pin_bytes.reverse();
    if pin_bytes.len() < 4 || pin_bytes.len() > 10 {
        return false;
    }
    let ascii: Vec<u8> = pin_bytes
        .iter()
        .map(|digit| {
            let index = remapped.iter().position(|v| v == digit).unwrap_or(0) as u8;
            index + 0x30
        })
        .collect();

    let mut first = Sha1::new();
    first.update(bn_as_byte_array(&conn.session.server_security_salt));
    first.update(&ascii);
    let first_digest: [u8; 20] = first.finalize().into();

    let mut second = Sha1::new();
    second.update(client_salt);
    second.update(bn_as_byte_array(&bn_from_binary(&first_digest)));
    let second_digest: [u8; 20] = second.finalize().into();

    bn_from_binary(&second_digest) == bn_from_binary(&client_hash)
}

/// `GenerateTotpPin` (`AuthSocket.cpp:1234-1261`) incl. its quirks:
/// big-endian step via `EndianConvertReverse`, dynamic truncation, a
/// native-endian byte swap of the truncated word, and `% 1_000_000`.
/// Returns `None` where the C++ returns `uint32(-1)`.
fn generate_totp_pin(secret: &str, interval: i32) -> Option<u32> {
    let key = base32_decode(secret)?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let step = (now / 30).wrapping_add(interval as u64);
    let digest = hmac_sha1(&key, &step.to_be_bytes());
    let offset = usize::from(digest[19] & 0xF);
    let pin_be = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    // C++ then applies EndianConvert (host LE → swap) before masking.
    let pin = (pin_be.swap_bytes() & 0x7FFF_FFFF) % 1_000_000;
    Some(pin)
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = Sha1::digest(key);
        key_block[..20].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha1::new();
    inner.update(key_block.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha1::new();
    outer.update(key_block.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner_digest);
    outer.finalize().into()
}

/// RFC 4648 base32 (the subset `Crypto::Encoding::Base32::Decode`
/// accepts): uppercase alphabet, optional `=` padding, ≤ 64 bytes out.
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    let mut out = Vec::new();
    for ch in input.bytes() {
        if ch == b'=' {
            break;
        }
        let value = match ch {
            b'A'..=b'Z' => u32::from(ch - b'A'),
            b'2'..=b'7' => u32::from(ch - b'2') + 26,
            _ => return None,
        };
        bits = (bits << 5) | value;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
        }
    }
    (out.len() <= 64).then_some(out)
}
