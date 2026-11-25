//! Login-database access for `realmd`.
//!
//! Design (migration document sections 5 and 12): one actor thread per database owns a
//! single synchronous `mysql` connection and runs jobs serially — the same serial shape
//! as the C++ `Database` worker for the login DB. All statements use prepared binds
//! (`?` placeholders), replacing the C++ `%s`-format + `escape_string` pattern.
//!
//! The query set is transcribed 1:1 from the C++ call sites:
//! `AuthSocket.cpp:286,297,379,707,730,732,745,754,853,1002,1303`,
//! `RealmList.cpp:167,251`, `realmd/Main.cpp:216-217`, `AccountMgr.cpp` (`CreateAccount`).
//! Deliberately excluded (Phase 1 charter): geolock pin updates and the `geoip`
//! country lookups (`AuthSocket.cpp:655,666,1337,1343`) — the geolock feature itself is
//! out of scope; the columns are still read where the logon flow needs them.

use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};

use mysql::prelude::Queryable;

use crate::DbError;

type Job = Box<dyn FnOnce(&mut mysql::Conn) + Send>;

type AccountTuple = (
    u32,
    u8,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    u8,
    u32,
    Option<String>,
    Option<u32>,
);
type RealmlistTuple = (
    u32,
    String,
    String,
    String,
    String,
    u32,
    u8,
    u8,
    u8,
    u8,
    f32,
    String,
);
type AllowedClientTuple = (u8, u8, u8, String, u16, String, String, Option<String>);

/// Handle to the login-DB actor thread. Clone-free by design: share via `Arc`.
pub struct LoginDb {
    tx: Sender<Job>,
}

/// `account` row as read by the logon challenge (`AuthSocket.cpp:297`).
/// Nullable text columns are collapsed to `""` like the C++ `Field::GetCppString`.
#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: u32,
    pub locked: u8,
    pub last_ip: String,
    pub v: String,
    pub s: String,
    /// TOTP secret (`security` varchar, NULL when unset).
    pub security: String,
    pub email_verif: u8,
    pub geolock_pin: u32,
    pub email: String,
    pub joindate_unix: u32,
}

/// `realmlist` row (`RealmList.cpp:167`).
#[derive(Debug, Clone)]
pub struct RealmlistRow {
    pub id: u32,
    pub name: String,
    pub address: String,
    pub local_address: String,
    pub local_subnet_mask: String,
    pub port: u32,
    pub icon: u8,
    pub realm_flags: u8,
    pub timezone: u8,
    pub allowed_security_level: u8,
    pub population: f32,
    pub realm_builds: String,
}

/// `allowed_clients` row (`RealmList.cpp:251`).
#[derive(Debug, Clone)]
pub struct AllowedClientRow {
    pub major_version: u8,
    pub minor_version: u8,
    pub bugfix_version: u8,
    pub hotfix_version: String,
    pub build: u16,
    pub os: String,
    pub platform: String,
    pub integrity_hash: String,
}

impl LoginDb {
    /// Connect and spawn the actor thread. Blocks until the initial connection succeeds
    /// or fails, so a bad DSN is a startup error (C++ `LoginDatabase.Initialize`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the URL is invalid or the connection is rejected.
    pub fn spawn(url: &str) -> Result<LoginDb, DbError> {
        let opts = mysql::Opts::from_url(url)?;
        let (ready_tx, ready_rx) = sync_channel::<Result<(), DbError>>(1);
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("login-db".to_string())
            .spawn(move || actor_loop(opts, rx, ready_tx))
            .map_err(|e| DbError::Driver(mysql::Error::IoError(e)))?;
        ready_rx
            .recv()
            .unwrap_or(Err(DbError::NoRow))
            .map(|()| LoginDb { tx })
    }

    /// Run one job on the actor connection and wait for its result.
    fn call<T, F>(&self, job: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut mysql::Conn) -> Result<T, DbError> + Send + 'static,
    {
        let (done_tx, done_rx) = sync_channel::<Result<T, DbError>>(1);
        self.tx
            .send(Box::new(move |conn| {
                let _ = done_tx.send(job(conn));
            }))
            .map_err(|_| DbError::NoRow)?;
        done_rx.recv().unwrap_or(Err(DbError::NoRow))
    }

    /// Active IP ban expiry for `ip`, if banned (`AuthSocket.cpp:286`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn ip_ban_unbandate(&self, ip: String) -> Result<Option<i64>, DbError> {
        self.call(move |conn| {
            Ok(conn.exec_first(
                "SELECT `unbandate` FROM `ip_banned` \
                 WHERE (`unbandate` = `bandate` OR `unbandate` > UNIX_TIMESTAMP()) AND `ip` = ?",
                (ip,),
            )?)
        })
    }

    /// Account lookup by (already normalized) username (`AuthSocket.cpp:297`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn account_by_username(&self, username: String) -> Result<Option<AccountRow>, DbError> {
        self.call(move |conn| {
            let row: Option<AccountTuple> = conn.exec_first(
                "SELECT `id`, `locked`, `last_ip`, `v`, `s`, `security`, `email_verif`, \
                 `geolock_pin`, `email`, UNIX_TIMESTAMP(`joindate`) \
                 FROM `account` WHERE `username` = ?",
                (username,),
            )?;
            Ok(row.map(|r| AccountRow {
                id: r.0,
                locked: r.1,
                last_ip: r.2,
                v: r.3.unwrap_or_default(),
                s: r.4.unwrap_or_default(),
                security: r.5.unwrap_or_default(),
                email_verif: r.6,
                geolock_pin: r.7,
                email: r.8.unwrap_or_default(),
                joindate_unix: r.9.unwrap_or_default(),
            }))
        })
    }

    /// Active account ban `(bandate, unbandate)` (`AuthSocket.cpp:379`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn active_account_ban(&self, account_id: u32) -> Result<Option<(i64, i64)>, DbError> {
        self.call(move |conn| {
            Ok(conn.exec_first(
                "SELECT `bandate`, `unbandate` FROM `account_banned` \
                 WHERE `id` = ? AND `active` = 1 \
                 AND (`unbandate` > UNIX_TIMESTAMP() OR `unbandate` = `bandate`) LIMIT 1",
                (account_id,),
            )?)
        })
    }

    /// The proof-success session stamp (`AuthSocket.cpp:707`, C++ `MustBeSync`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn update_session_on_proof(
        &self,
        username: String,
        sessionkey_hex: String,
        last_ip: String,
        locale: u32,
        os: String,
        platform: String,
    ) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.exec_drop(
                "UPDATE `account` SET `sessionkey` = ?, `last_ip` = ?, `last_login` = NOW(), \
                 `locale` = ?, `failed_logins` = 0, `os` = ?, `platform` = ? \
                 WHERE `username` = ?",
                (sessionkey_hex, last_ip, locale, os, platform, username),
            )?;
            Ok(())
        })
    }

    /// Increment `failed_logins` and read the new count (`AuthSocket.cpp:730,732`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn record_failed_login(&self, username: String) -> Result<Option<(u32, u32)>, DbError> {
        self.call(move |conn| {
            conn.exec_drop(
                "UPDATE `account` SET `failed_logins` = `failed_logins` + 1 \
                 WHERE `username` = ?",
                (username.clone(),),
            )?;
            Ok(conn.exec_first(
                "SELECT `id`, `failed_logins` FROM `account` WHERE `username` = ?",
                (username,),
            )?)
        })
    }

    /// Wrong-password autoban, account variant (`AuthSocket.cpp:745`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn insert_account_autoban(&self, account_id: u32, ban_seconds: u32) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.exec_drop(
                "INSERT INTO `account_banned` \
                 (`id`, `bandate`, `unbandate`, `bannedby`, `banreason`, `active`, `realm`) \
                 VALUES (?, UNIX_TIMESTAMP(), UNIX_TIMESTAMP() + ?, \
                 'MaNGOS realmd', 'Failed login autoban', 1, 1)",
                (account_id, ban_seconds),
            )?;
            Ok(())
        })
    }

    /// Wrong-password autoban, IP variant (`AuthSocket.cpp:754`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn insert_ip_autoban(&self, ip: String, ban_seconds: u32) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.exec_drop(
                "INSERT INTO `ip_banned` \
                 VALUES (?, UNIX_TIMESTAMP(), UNIX_TIMESTAMP() + ?, \
                 'MaNGOS realmd', 'Failed login autoban')",
                (ip, ban_seconds),
            )?;
            Ok(())
        })
    }

    /// Reconnect-challenge session key (`AuthSocket.cpp:853`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn sessionkey_by_username(
        &self,
        username: String,
    ) -> Result<Option<(String, u32)>, DbError> {
        self.call(move |conn| {
            let row: Option<(Option<String>, u32)> = conn.exec_first(
                "SELECT `sessionkey`, `id` FROM `account` WHERE `username` = ?",
                (username,),
            )?;
            Ok(row.map(|(key, id)| (key.unwrap_or_default(), id)))
        })
    }

    /// Per-realm character count (`AuthSocket.cpp:1002`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn num_chars(&self, realm_id: u32, account_id: u32) -> Result<Option<u8>, DbError> {
        self.call(move |conn| {
            Ok(conn.exec_first(
                "SELECT `numchars` FROM `realmcharacters` \
                 WHERE `realmid` = ? AND `acctid` = ?",
                (realm_id, account_id),
            )?)
        })
    }

    /// `account_access` rows `(gmlevel, realm_id)` (`AuthSocket.cpp:1303`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn account_access(&self, account_id: u32) -> Result<Vec<(u8, i32)>, DbError> {
        self.call(move |conn| {
            Ok(conn.exec(
                "SELECT `gmlevel`, `RealmID` FROM `account_access` WHERE `id` = ?",
                (account_id,),
            )?)
        })
    }

    /// Visible realms ordered by name (`RealmList.cpp:167`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn realmlist(&self) -> Result<Vec<RealmlistRow>, DbError> {
        self.call(move |conn| {
            let rows: Vec<RealmlistTuple> = conn.query(
                "SELECT `id`, `name`, `address`, `localAddress`, `localSubnetMask`, `port`, \
                 `icon`, `realmflags`, `timezone`, `allowedSecurityLevel`, `population`, \
                 `realmbuilds` FROM `realmlist` WHERE (`realmflags` & 1) = 0 ORDER BY `name`",
            )?;
            Ok(rows
                .into_iter()
                .map(|r| RealmlistRow {
                    id: r.0,
                    name: r.1,
                    address: r.2,
                    local_address: r.3,
                    local_subnet_mask: r.4,
                    port: r.5,
                    icon: r.6,
                    realm_flags: r.7,
                    timezone: r.8,
                    allowed_security_level: r.9,
                    population: r.10,
                    realm_builds: r.11,
                })
                .collect())
        })
    }

    /// Allowed client builds (`RealmList.cpp:251`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn allowed_clients(&self) -> Result<Vec<AllowedClientRow>, DbError> {
        self.call(move |conn| {
            let rows: Vec<AllowedClientTuple> = conn.query(
                "SELECT `major_version`, `minor_version`, `bugfix_version`, \
                     `hotfix_version`, `build`, `os`, `platform`, `integrity_hash` \
                     FROM `allowed_clients`",
            )?;
            Ok(rows
                .into_iter()
                .map(|r| AllowedClientRow {
                    major_version: r.0,
                    minor_version: r.1,
                    bugfix_version: r.2,
                    hotfix_version: r.3,
                    build: r.4,
                    os: r.5,
                    platform: r.6,
                    integrity_hash: r.7.unwrap_or_default(),
                })
                .collect())
        })
    }

    /// Startup ban cleanup (`realmd/Main.cpp:216-217`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn cleanup_expired_bans(&self) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.query_drop(
                "UPDATE `account_banned` SET `active` = 0 \
                 WHERE `unbandate` <= UNIX_TIMESTAMP() AND `unbandate` <> `bandate`",
            )?;
            conn.query_drop(
                "DELETE FROM `ip_banned` \
                 WHERE `unbandate` <= UNIX_TIMESTAMP() AND `unbandate` <> `bandate`",
            )?;
            Ok(())
        })
    }

    /// Applied migration names + current schema name, for the boot-time
    /// migration gate (`Database::CheckRequiredMigrations`, `Database.cpp:570`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn applied_migrations(&self) -> Result<(Vec<String>, String), DbError> {
        self.call(move |conn| {
            let names: Vec<String> = conn.query("SELECT * FROM `migrations`")?;
            let db_name: Option<String> = conn.query_first("SELECT DATABASE()")?;
            Ok((names, db_name.ok_or(DbError::NoRow)?))
        })
    }

    /// Whether the `geoip` table has any row — the startup sanity check
    /// realmd runs when `GeoLocking` is enabled (`realmd/Main.cpp:185`).
    /// The geolock lookups themselves stay out of Phase 1 scope.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn geoip_table_nonempty(&self) -> Result<bool, DbError> {
        self.call(move |conn| {
            let row: Option<u8> = conn.query_first("SELECT 1 FROM `geoip` LIMIT 1")?;
            Ok(row.is_some())
        })
    }

    /// Account id by username, if any (`AccountMgr::GetId`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn account_id(&self, username: String) -> Result<Option<u32>, DbError> {
        self.call(move |conn| {
            Ok(conn.exec_first(
                "SELECT `id` FROM `account` WHERE `username` = ?",
                (username,),
            )?)
        })
    }

    /// Create an account with precomputed SRP `v`/`s` hex (`AccountMgr::CreateAccount`),
    /// including the `realmcharacters` seed REPLACE. Username must already be
    /// normalized (uppercased) by the caller, as in C++.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn create_account(
        &self,
        username: String,
        v_hex: String,
        s_hex: String,
    ) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.exec_drop(
                "INSERT INTO `account` (`username`, `v`, `s`, `joindate`) \
                 VALUES (?, ?, ?, NOW())",
                (username, v_hex, s_hex),
            )?;
            conn.query_drop(
                "REPLACE INTO `realmcharacters` (`realmid`, `acctid`, `numchars`) \
                 SELECT `realmlist`.`id`, `account`.`id`, 0 \
                 FROM `realmlist`, `account` \
                 LEFT JOIN `realmcharacters` ON `acctid` = `account`.`id` \
                 WHERE `acctid` IS NULL",
            )?;
            Ok(())
        })
    }

    /// Delete an account row (test-fixture cleanup only; not a C++ realmd path).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn delete_account_row(&self, username: String) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.exec_drop("DELETE FROM `account` WHERE `username` = ?", (username,))?;
            Ok(())
        })
    }
}

// By-value is required: the actor thread owns the options (for reconnects) and both channels.
#[allow(clippy::needless_pass_by_value)]
fn actor_loop(opts: mysql::Opts, rx: Receiver<Job>, ready: SyncSender<Result<(), DbError>>) {
    let mut conn = match mysql::Conn::new(opts.clone()) {
        Ok(conn) => {
            let _ = ready.send(Ok(()));
            conn
        }
        Err(e) => {
            let _ = ready.send(Err(DbError::Driver(e)));
            return;
        }
    };
    while let Ok(job) = rx.recv() {
        if conn.ping().is_err()
            && let Ok(fresh) = mysql::Conn::new(opts.clone())
        {
            conn = fresh;
        }
        job(&mut conn);
    }
}
