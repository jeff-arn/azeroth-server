//! MySQL access for the Rust port.
//!
//! Phase 0 seed (`docs/RUST_MIGRATION_ARCHITECTURE.md` section 5): a
//! connection constructor and the smallest useful read, so the
//! `TEST_MYSQL_URL`-gated live-test harness has something real to prove.
//! The Phase 1 design — one actor per database with per-character
//! serial-lane ordering, prepared binds, and the migration gate —
//! replaces this surface; do not build call sites on it yet.
//!
//! Driver choice: the pure-Rust `mysql` crate, per the migration
//! document's recommendation (section 11) to prefer pure-Rust MySQL +
//! rustls over recreating the C++ find-module/`libmysqlclient` pain;
//! it also keeps a musl-static build possible. Owner-overridable.
//!
//! Phase 1: [`login::LoginDb`] is the per-DB actor for the login
//! database; [`connect`]/[`Conn::count_migrations`] remain as the
//! live-test probe. Phase 3 adds the generic [`actor::DbActor`],
//! the [`migrations`] boot gate, [`connection_info::ConnectionInfo`],
//! and [`world_auth::WorldAuthDb`] for mangosd.

use mysql::prelude::Queryable;

pub mod actor;
pub mod character_load;
pub mod characters;
pub mod connection_info;
pub mod login;
pub mod migrations;
// Field conversions wrap/round like C++ atol/atof by design.
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_precision_loss)]
pub mod row;
pub mod world_auth;

pub use actor::DbActor;
pub use characters::CharacterDb;
pub use connection_info::ConnectionInfo;
pub use login::LoginDb;
pub use world_auth::WorldAuthDb;

/// Errors from the MySQL driver or URL parsing.
#[derive(Debug)]
pub enum DbError {
    Driver(mysql::Error),
    Url(mysql::UrlError),
    /// A query that must yield one row yielded none.
    NoRow,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Driver(e) => write!(f, "mysql error: {e}"),
            Self::Url(e) => write!(f, "mysql url error: {e}"),
            Self::NoRow => write!(f, "expected one row, got none"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Driver(e) => Some(e),
            Self::Url(e) => Some(e),
            Self::NoRow => None,
        }
    }
}

impl From<mysql::Error> for DbError {
    fn from(error: mysql::Error) -> Self {
        Self::Driver(error)
    }
}

impl From<mysql::UrlError> for DbError {
    fn from(error: mysql::UrlError) -> Self {
        Self::Url(error)
    }
}

/// A single synchronous connection. Phase 1 replaces this with the
/// per-DB actor; see the crate docs.
pub struct Conn(mysql::Conn);

/// Open a connection from a `mysql://user:pass@host:port/db` URL.
///
/// # Errors
/// Returns [`DbError`] if the URL is invalid or the server rejects the
/// connection or credentials.
pub fn connect(url: &str) -> Result<Conn, DbError> {
    let opts = mysql::Opts::from_url(url)?;
    Ok(Conn(mysql::Conn::new(opts)?))
}

impl Conn {
    /// Number of rows in the `migrations` table of the connected
    /// database — the same table the C++ migration gate checks at boot
    /// (`Database.cpp:570` per the migration document, section 5).
    ///
    /// # Errors
    /// Returns [`DbError`] if the query fails (e.g. table missing).
    pub fn count_migrations(&mut self) -> Result<i64, DbError> {
        let count: Option<i64> = self.0.query_first("SELECT COUNT(*) FROM `migrations`")?;
        count.ok_or(DbError::NoRow)
    }
}
