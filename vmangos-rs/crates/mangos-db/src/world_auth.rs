//! Login-DB access from the world server's side: the
//! `CMSG_AUTH_SESSION` account query (`WorldSocket.cpp:263-282`),
//! the `last_ip` write (`:384`), and the `ip_banned` check behind
//! `sAccountMgr.IsIPBanned` (same predicate the Rust realmd uses,
//! `AuthSocket.cpp:286`). mangosd owns its own connections to the
//! login DB, separate from realmd's (architecture section 3.1:
//! the contract is the shared schema, not shared connections).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::actor::DbActor;

/// One row of the world-auth account query. Field order mirrors
/// the C++ `SELECT` (indices 0-13 there; `v`/`s` are selected by
/// C++ but never read on this path, so they are omitted here).
/// Nullable text collapses to `""` like `Field::GetCppString`;
/// `security` is `None` when the `account_access` join missed
/// (C++ null → `SEC_PLAYER`, `WorldSocket.cpp:311`).
#[derive(Clone, Debug)]
pub struct WorldAuthAccount {
    pub id: u32,
    pub security: Option<u32>,
    pub sessionkey_hex: String,
    pub last_ip: String,
    /// `mutetime` is a signed bigint; C++ reads it with
    /// `GetUInt64`, wrapping — same cast here.
    pub mutetime: u64,
    pub locale: u8,
    pub os: String,
    pub platform: String,
    pub flags: u32,
    pub email: String,
    pub email_verif: bool,
    /// `ab.unbandate > UNIX_TIMESTAMP() OR ab.unbandate =
    /// ab.bandate`; NULL (no ban row) reads false like
    /// `Field::GetBool` on NULL.
    pub banned: bool,
}

type WorldAuthTuple = (
    u32,
    Option<u32>,
    Option<String>,
    String,
    i64,
    u8,
    Option<String>,
    Option<String>,
    u32,
    Option<String>,
    u8,
    Option<i8>,
);

/// Handle to mangosd's login-DB actor for world auth.
pub struct WorldAuthDb {
    actor: DbActor,
}

impl WorldAuthDb {
    /// Connect and spawn (see [`DbActor::spawn`]).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on a bad URL or rejected connection.
    pub fn spawn(url: &str) -> Result<WorldAuthDb, DbError> {
        Ok(WorldAuthDb {
            actor: DbActor::spawn("login-db-world", url)?,
        })
    }

    #[must_use]
    pub fn actor(&self) -> &DbActor {
        &self.actor
    }

    /// The `_HandleAuthSession` account query with prepared binds
    /// (C++ interpolates an escaped string; binds are the
    /// established Rust replacement). `DATEDIFF(NOW(), last_login)
    /// < 1` means an account that has not logged in through realmd
    /// today does not exist as far as world auth is concerned.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn lookup_world_account(
        &self,
        realm_id: u32,
        account: String,
    ) -> Result<Option<WorldAuthAccount>, DbError> {
        self.actor.call(move |conn| {
            let row: Option<WorldAuthTuple> = conn.exec_first(
                "SELECT a.`id`, aa.`gmLevel`, a.`sessionkey`, a.`last_ip`, \
                        a.`mutetime`, a.`locale`, a.`os`, a.`platform`, a.`flags`, \
                        a.`email`, a.`email_verif`, \
                        ab.`unbandate` > UNIX_TIMESTAMP() OR ab.`unbandate` = ab.`bandate` \
                 FROM `account` a \
                 LEFT JOIN `account_access` aa ON a.`id` = aa.`id` AND aa.`RealmID` IN (-1, ?) \
                 LEFT JOIN `account_banned` ab ON a.`id` = ab.`id` AND ab.`active` = 1 \
                 WHERE a.`username` = ? AND DATEDIFF(NOW(), a.`last_login`) < 1 \
                 ORDER BY aa.`RealmID` DESC LIMIT 1",
                (realm_id, account),
            )?;
            Ok(row.map(|r| WorldAuthAccount {
                id: r.0,
                security: r.1,
                sessionkey_hex: r.2.unwrap_or_default(),
                last_ip: r.3,
                mutetime: r.4 as u64,
                locale: r.5,
                os: r.6.unwrap_or_default(),
                platform: r.7.unwrap_or_default(),
                flags: r.8,
                email: r.9.unwrap_or_default(),
                email_verif: r.10 != 0,
                banned: r.11.unwrap_or(0) != 0,
            }))
        })
    }

    /// `account.email_verif` by id (offline trial-
    /// restriction checks); `None` when the account
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn account_email_verified(&self, account_id: u32) -> Result<Option<bool>, DbError> {
        self.actor.call(move |conn| {
            use mysql::prelude::Queryable;
            let row: Option<u8> = conn.exec_first(
                "SELECT `email_verif` FROM `account` WHERE `id` = ?",
                (account_id,),
            )?;
            Ok(row.map(|value| value != 0))
        })
    }

    /// `UPDATE account SET last_ip = ? WHERE username = ?`
    /// (`WorldSocket.cpp:384`; C++ binds address first).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn update_last_ip(&self, account: String, ip: String) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "UPDATE `account` SET `last_ip` = ? WHERE `username` = ?",
                (ip, account),
            )?;
            Ok(())
        })
    }

    /// `UPDATE realmlist SET population = ? WHERE id = ?`
    /// (`World::AddSession_`, `World.cpp:332`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn set_realm_population(&self, realm_id: u32, population: f32) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "UPDATE `realmlist` SET `population` = ? WHERE `id` = ?",
                (population, realm_id),
            )?;
            Ok(())
        })
    }

    /// Realm online publish (`Master.cpp:207`): clear the
    /// OFFLINE realmflag (0x02), zero population, write the
    /// accepted-builds string.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn set_realm_online(&self, realm_id: u32, builds: String) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "UPDATE `realmlist` SET `realmflags` = `realmflags` & ~(2), `population` = 0, `realmbuilds` = ? WHERE `id` = ?",
                (builds, realm_id),
            )?;
            Ok(())
        })
    }

    /// Realm offline publish at shutdown (`Master.cpp:317`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn set_realm_offline(&self, realm_id: u32) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "UPDATE `realmlist` SET `realmflags` = `realmflags` | 2 WHERE `id` = ?",
                (realm_id,),
            )?;
            Ok(())
        })
    }

    /// The boot `uptime` row (`World.cpp:1756`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn insert_uptime(
        &self,
        realm_id: u32,
        start_time: u64,
        start_string: String,
        revision: String,
    ) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "INSERT INTO `uptime` (`realmid`, `starttime`, `startstring`, `revision`) VALUES(?, ?, ?, ?)",
                (realm_id, start_time, start_string, revision),
            )?;
            Ok(())
        })
    }

    /// The periodic `uptime` refresh (`World.cpp:2024`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn update_uptime(
        &self,
        realm_id: u32,
        start_time: u64,
        uptime_secs: u32,
        online: u32,
        max_online: u32,
    ) -> Result<(), DbError> {
        self.actor.call(move |conn| {
            conn.exec_drop(
                "UPDATE `uptime` SET `uptime` = ?, `onlineplayers` = ?, `maxplayers` = ? WHERE `realmid` = ? AND `starttime` = ?",
                (uptime_secs, online, max_online, realm_id, start_time),
            )?;
            Ok(())
        })
    }

    /// Whether `ip` has an active ban row.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn is_ip_banned(&self, ip: String) -> Result<bool, DbError> {
        self.actor.call(move |conn| {
            let found: Option<i64> = conn.exec_first(
                "SELECT `unbandate` FROM `ip_banned` \
                 WHERE (`unbandate` = `bandate` OR `unbandate` > UNIX_TIMESTAMP()) AND `ip` = ?",
                (ip,),
            )?;
            Ok(found.is_some())
        })
    }
}
