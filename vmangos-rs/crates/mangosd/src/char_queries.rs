//! The character-DB seam for session handlers. Methods are
//! blocking (they run inside `spawn_blocking` jobs, mirroring
//! the C++ async query callbacks); the `mangos-db` adapter
//! lives in the binary, tests use in-memory fakes.

use mangos_db::DbError;
use mangos_db::character_load::CharacterLoad;
use mangos_db::characters::{AccountDataEntry, CharEnumRow, NameQueryRow};

pub trait CharQueries: Send + Sync {
    /// The `CMSG_CHAR_ENUM` rows for one account.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn char_enum(&self, account_id: u32) -> Result<Vec<CharEnumRow>, DbError>;

    /// The full 21-query read-only character load;
    /// `Ok(None)` = no such character.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn load_character(&self, guid_low: u32) -> Result<Option<CharacterLoad>, DbError>;

    /// The `CMSG_NAME_QUERY` fallback identity row;
    /// `Ok(None)` = no such character.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn name_query(&self, guid_low: u32) -> Result<Option<NameQueryRow>, DbError>;

    /// Global `account_data` rows (loaded at world auth).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn global_account_data(&self, account_id: u32) -> Result<Vec<AccountDataEntry>, DbError>;

    /// Tutorial flags; `None` = no row yet.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn tutorials(&self, account_id: u32) -> Result<Option<[u32; 8]>, DbError>;
}
