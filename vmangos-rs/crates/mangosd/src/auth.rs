//! `CMSG_AUTH_SESSION` decision logic
//! (`WorldSocket::_HandleAuthSession`, `WorldSocket.cpp:223-440`)
//! split into a pure, DB-free core plus the [`AuthBackend`] seam
//! the `mangos-db` world-auth actor implements.
//!
//! Query transcription (field order is load-bearing):
//! ```sql
//! SELECT a.id, aa.gmLevel, a.sessionkey, a.last_ip, a.v, a.s,
//!        a.mutetime, a.locale, a.os, a.platform, a.flags, a.email,
//!        a.email_verif,
//!        ab.unbandate > UNIX_TIMESTAMP() OR ab.unbandate = ab.bandate
//! FROM account a
//! LEFT JOIN account_access aa ON a.id = aa.id AND aa.RealmID IN (-1, ?)
//! LEFT JOIN account_banned ab ON a.id = ab.id AND ab.active = 1
//! WHERE a.username = ? AND DATEDIFF(NOW(), a.last_login) < 1
//! ORDER BY aa.RealmID DESC LIMIT 1
//! ```
//! `v`/`s` are selected by C++ but not read on this path; the Rust
//! row keeps them out.

use std::future::Future;
use std::pin::Pin;

use num_bigint::BigUint;

use mangos_db::DbError;
use mangos_protocol::world::{
    AuthSessionPacket, auth_response, auth_session_digest_matches, session_key_from_hex,
};

use crate::{ServerConfig, is_acceptable_client_build, security};

/// The account row lives with its query in `mangos-db`
/// (the `AccountRow` precedent); re-exported for callers here.
pub use mangos_db::world_auth::WorldAuthAccount;

/// `ClientOSType` values assigned at `WorldSocket.cpp:387-396`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientOs {
    Win,
    Mac,
}

/// `ClientPlatformType` values assigned at `WorldSocket.cpp:398-407`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientPlatform {
    X86,
    Ppc,
}

/// Everything the C++ copies onto the new `WorldSession` before
/// `sWorld.AddSession` (`WorldSocket.cpp:417-433`). The two
/// character-DB reads the C++ performs synchronously here —
/// `LoadGlobalAccountData` and `LoadTutorialsData` — land with the
/// character-load slice; they do not gate session creation.
#[derive(Clone, Debug)]
pub struct AcceptedAuth {
    pub account_id: u32,
    pub username: String,
    pub security: u8,
    pub mutetime: u64,
    /// Locale clamped like `WorldSocket.cpp:320-322` (`>= MAX_LOCALE`
    /// → `LOCALE_enUS` = 0).
    pub locale: u8,
    pub client_build: u32,
    pub account_flags: u32,
    pub client_os: ClientOs,
    pub client_platform: ClientPlatform,
    pub verified_email: bool,
    pub email: String,
    pub session_key: BigUint,
}

/// Outcome of [`authenticate`], in the C++ `HandlerResult` shape:
/// every variant except `Accept` ends the connection.
#[derive(Debug)]
pub enum AuthOutcome {
    /// Send `SMSG_AUTH_RESPONSE` with this code, then close.
    Reject(u8),
    /// Close without a packet (`HandlerResult::Fail` with no send:
    /// empty session key at `WorldSocket.cpp:316-317`).
    FailSilently,
    /// Digest verified, so the C++ has already issued the
    /// `last_ip` write (`WorldSocket.cpp:384-385`), but the stored
    /// OS/platform strings are unrecognized (`:387-407`): close
    /// without a packet. The caller performs the write to keep
    /// this function pure.
    BadOsOrPlatform,
    /// Authenticated. The caller performs the `last_ip` write
    /// before creating the session, matching C++ order.
    Accept(Box<AcceptedAuth>),
}

impl AuthOutcome {
    /// Whether the C++ reaches the `last_ip` UPDATE for this
    /// outcome (digest verified before the write).
    #[must_use]
    pub fn updates_last_ip(&self) -> bool {
        matches!(self, AuthOutcome::Accept(_) | AuthOutcome::BadOsOrPlatform)
    }
}

/// Inputs to [`authenticate`] that come from outside the packet.
pub struct AuthContext<'a> {
    pub config: &'a ServerConfig,
    /// This socket's challenge seed (`m_authSeed`).
    pub server_seed: u32,
    /// Remote address string as C++ formats it
    /// (`GetRemoteIpString`).
    pub remote_ip: &'a str,
    /// Whether `remote_ip` is one of this host's own addresses,
    /// which bypasses the `last_ip` match
    /// (`WorldSocket.cpp:300`, `GetServerAddresses`).
    pub remote_is_server_address: bool,
    /// `sAccountMgr.IsIPBanned(remote_ip)`, evaluated by the
    /// caller so the IP check at `WorldSocket.cpp:330` stays in
    /// the C++ order (after the account row exists).
    pub ip_banned: bool,
}

/// The `_HandleAuthSession` decision chain after the packet is
/// parsed and the account query has run. Check order preserved:
/// version → account exists → `last_ip` → key non-empty → ban →
/// security limit → digest → OS → platform.
#[must_use]
pub fn authenticate(
    packet: &AuthSessionPacket,
    account: Option<&WorldAuthAccount>,
    ctx: &AuthContext<'_>,
) -> AuthOutcome {
    if !is_acceptable_client_build(packet.client_build) {
        return AuthOutcome::Reject(auth_response::VERSION_MISMATCH);
    }
    let Some(account) = account else {
        return AuthOutcome::Reject(auth_response::UNKNOWN_ACCOUNT);
    };
    if account.last_ip != ctx.remote_ip && !ctx.remote_is_server_address {
        return AuthOutcome::Reject(auth_response::FAILED);
    }

    let security = account.security.map_or(security::PLAYER, |gm| {
        let gm = gm.min(u32::from(security::ADMINISTRATOR));
        gm as u8
    });

    let Some(session_key) = session_key_from_hex(&account.sessionkey_hex) else {
        return AuthOutcome::FailSilently;
    };
    if session_key == BigUint::ZERO {
        return AuthOutcome::FailSilently;
    }

    if account.banned || ctx.ip_banned {
        return AuthOutcome::Reject(auth_response::BANNED);
    }

    let allowed = ctx.config.player_security_limit;
    if allowed > security::PLAYER && security < allowed {
        return AuthOutcome::Reject(auth_response::UNAVAILABLE);
    }

    if !auth_session_digest_matches(packet, ctx.server_seed, &session_key) {
        return AuthOutcome::Reject(auth_response::FAILED);
    }

    // Digest verified: C++ writes last_ip here, then validates
    // the stored OS/platform strings.
    let client_os = match account.os.as_str() {
        "Win" => ClientOs::Win,
        "OSX" => ClientOs::Mac,
        _ => return AuthOutcome::BadOsOrPlatform,
    };
    let client_platform = match account.platform.as_str() {
        "x86" => ClientPlatform::X86,
        "PPC" if client_os == ClientOs::Mac => ClientPlatform::Ppc,
        _ => return AuthOutcome::BadOsOrPlatform,
    };

    AuthOutcome::Accept(Box::new(AcceptedAuth {
        account_id: account.id,
        username: packet.account.clone(),
        security,
        mutetime: account.mutetime,
        locale: if account.locale >= MAX_LOCALE {
            0
        } else {
            account.locale
        },
        client_build: packet.client_build,
        account_flags: account.flags,
        client_os,
        client_platform,
        verified_email: account.email_verif || account.email.is_empty(),
        email: account.email.clone(),
        session_key,
    }))
}

/// `MAX_LOCALE` (`Common.h`, locale enum count at 1.12.1).
const MAX_LOCALE: u8 = 9;

/// The login-DB seam for world auth. Implemented by the
/// `mangos-db` world-auth actor; tests use an in-memory fake.
pub trait AuthBackend: Send + Sync {
    /// Run the account query transcribed at the top of this
    /// module for `account` on the given realm.
    fn lookup_world_account<'a>(
        &'a self,
        realm_id: u32,
        account: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<WorldAuthAccount>, DbError>> + Send + 'a>>;

    /// `UPDATE account SET last_ip = ? WHERE username = ?`
    /// (`WorldSocket.cpp:384`).
    fn update_last_ip<'a>(
        &'a self,
        account: &'a str,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>>;

    /// Whether `ip` has an active `ip_ban` row.
    fn is_ip_banned<'a>(
        &'a self,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_config::Config;
    use mangos_protocol::world::expected_auth_session_digest;

    // Independent hashlib vector (see world_packets tests):
    // K bytes [0x02, 0x01] = hex "0102", seeds 0x11223344/0x55667788.
    const GOOD_DIGEST: [u8; 20] = [
        111, 41, 242, 235, 194, 180, 210, 59, 225, 220, 130, 166, 201, 126, 111, 160, 119, 134, 74,
        80,
    ];

    fn packet() -> AuthSessionPacket {
        AuthSessionPacket {
            client_build: 5875,
            server_id: 1,
            account: "TESTUSER".to_string(),
            client_seed: 0x1122_3344,
            digest: GOOD_DIGEST,
            addon_data: Vec::new(),
        }
    }

    fn account() -> WorldAuthAccount {
        WorldAuthAccount {
            id: 42,
            security: None,
            sessionkey_hex: "0102".to_string(),
            last_ip: "127.0.0.1".to_string(),
            mutetime: 0,
            locale: 0,
            os: "Win".to_string(),
            platform: "x86".to_string(),
            flags: 0,
            email: String::new(),
            email_verif: false,
            banned: false,
        }
    }

    fn ctx(config: &ServerConfig) -> AuthContext<'_> {
        AuthContext {
            config,
            server_seed: 0x5566_7788,
            remote_ip: "127.0.0.1",
            remote_is_server_address: false,
            ip_banned: false,
        }
    }

    fn config() -> ServerConfig {
        ServerConfig::from_config(&Config::parse(b"RealmID = 1\n"))
    }

    #[test]
    fn digest_vector_is_consistent_with_protocol_crate() {
        let key = session_key_from_hex("0102").unwrap();
        assert_eq!(
            expected_auth_session_digest("TESTUSER", 0x1122_3344, 0x5566_7788, &key),
            GOOD_DIGEST
        );
    }

    #[test]
    fn accept_path() {
        let cfg = config();
        let out = authenticate(&packet(), Some(&account()), &ctx(&cfg));
        assert!(out.updates_last_ip());
        let AuthOutcome::Accept(accepted) = out else {
            panic!("expected Accept, got {out:?}");
        };
        assert_eq!(accepted.account_id, 42);
        assert_eq!(accepted.security, security::PLAYER);
        assert_eq!(accepted.client_os, ClientOs::Win);
        assert!(accepted.verified_email); // empty email counts as verified
    }

    #[test]
    fn version_then_unknown_account_order() {
        let cfg = config();
        let mut p = packet();
        p.client_build = 5302;
        let out = authenticate(&p, None, &ctx(&cfg));
        assert!(matches!(
            out,
            AuthOutcome::Reject(c) if c == auth_response::VERSION_MISMATCH
        ));
        let out = authenticate(&packet(), None, &ctx(&cfg));
        assert!(matches!(
            out,
            AuthOutcome::Reject(c) if c == auth_response::UNKNOWN_ACCOUNT
        ));
    }

    #[test]
    fn last_ip_mismatch_and_server_address_bypass() {
        let cfg = config();
        let mut acct = account();
        acct.last_ip = "10.0.0.9".to_string();
        let out = authenticate(&packet(), Some(&acct), &ctx(&cfg));
        assert!(matches!(out, AuthOutcome::Reject(c) if c == auth_response::FAILED));
        assert!(!out.updates_last_ip());

        let mut bypass = ctx(&cfg);
        bypass.remote_is_server_address = true;
        let out = authenticate(&packet(), Some(&acct), &bypass);
        assert!(matches!(out, AuthOutcome::Accept(_)));
    }

    #[test]
    fn empty_key_fails_silently_before_ban_check() {
        let cfg = config();
        let mut acct = account();
        acct.sessionkey_hex = String::new();
        acct.banned = true;
        let out = authenticate(&packet(), Some(&acct), &ctx(&cfg));
        assert!(matches!(out, AuthOutcome::FailSilently));
    }

    #[test]
    fn banned_and_security_limit_precede_digest() {
        let cfg = config();
        let mut acct = account();
        acct.banned = true;
        let mut p = packet();
        p.digest = [0; 20];
        let out = authenticate(&p, Some(&acct), &ctx(&cfg));
        assert!(matches!(out, AuthOutcome::Reject(c) if c == auth_response::BANNED));

        let mut limited = config();
        limited.player_security_limit = 3;
        let out = authenticate(&p, Some(&account()), &ctx(&limited));
        assert!(matches!(
            out,
            AuthOutcome::Reject(c) if c == auth_response::UNAVAILABLE
        ));

        let out = authenticate(&p, Some(&account()), &ctx(&cfg));
        assert!(matches!(out, AuthOutcome::Reject(c) if c == auth_response::FAILED));
        assert!(!out.updates_last_ip());
    }

    #[test]
    fn bad_os_after_digest_still_writes_last_ip() {
        let cfg = config();
        let mut acct = account();
        acct.os = "Lnx".to_string();
        let out = authenticate(&packet(), Some(&acct), &ctx(&cfg));
        assert!(matches!(out, AuthOutcome::BadOsOrPlatform));
        assert!(out.updates_last_ip());
    }

    #[test]
    fn security_clamps_to_administrator() {
        let cfg = config();
        let mut acct = account();
        acct.security = Some(99);
        let AuthOutcome::Accept(accepted) = authenticate(&packet(), Some(&acct), &ctx(&cfg)) else {
            panic!("expected Accept");
        };
        assert_eq!(accepted.security, security::ADMINISTRATOR);
    }
}
