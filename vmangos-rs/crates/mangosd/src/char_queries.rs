//! The character-DB seam for session handlers. Methods are
//! blocking (they run inside `spawn_blocking` jobs, mirroring
//! the C++ async query callbacks); the `mangos-db` adapter
//! lives in the binary, tests use in-memory fakes.

use mangos_db::DbError;
use mangos_db::character_load::CharacterLoad;
use mangos_db::characters::{
    AccountDataEntry, CharEnumRow, CharacterOwner, DeletePlan, NameQueryRow, NewCharacter,
};

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

    /// Name-in-use existence (`GetPlayerGuidByName`
    /// create gate).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn name_in_use(&self, name: &str) -> Result<bool, DbError>;

    /// Live character count for an account
    /// (`_UpdateRealmCharCount` recount).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn character_count(&self, account_id: u32) -> Result<u32, DbError>;

    /// Delete-time identity (account, name, level);
    /// `Ok(None)` = no such character.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn character_owner(&self, guid_low: u32) -> Result<Option<CharacterOwner>, DbError>;

    /// `Player::SaveNewPlayer` insert; returns the new
    /// low guid.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn create_character(&self, new: &NewCharacter) -> Result<u32, DbError>;

    /// `Player::DeleteFromDB` DB half for a resolved
    /// plan (live side effects already applied).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_character(&self, guid_low: u32, plan: &DeletePlan) -> Result<(), DbError>;

    /// Forced-rename gate + update; `Ok(None)` = gate
    /// miss, `Ok(Some(old))` = renamed.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn rename_character(
        &self,
        guid_low: u32,
        account_id: u32,
        new_name: &str,
    ) -> Result<Option<String>, DbError>;

    /// `Player::DeleteOldCharacters` candidates:
    /// `(guid, deleted_account)` pairs, at most two.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn old_deleted_characters(&self, keep_days: u32, now: u64) -> Result<Vec<(u32, u32)>, DbError>;

    /// `REPLACE INTO realmcharacters` on the login DB
    /// (create handler + `_UpdateRealmCharCount`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn sync_realm_char_count(&self, account_id: u32, count: u32) -> Result<(), DbError>;

    /// Persist a resurrectable corpse
    /// (`Corpse::SaveToDB`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn save_corpse(&self, row: &mangos_db::corpse::CorpseRow) -> Result<(), DbError>;

    /// Delete a player's persisted corpse rows
    /// (`Corpse::DeleteFromDB`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_corpse(&self, player_low: u32) -> Result<(), DbError>;

    /// Boot corpse load with owner appearance
    /// (`ObjectMgr::LoadCorpses`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn load_corpses(&self) -> Result<Vec<mangos_db::corpse::LoadedCorpse>, DbError>;

    /// Highest persisted corpse guid
    /// (`GenerateCorpseLowGuid` seed).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn max_corpse_guid(&self) -> Result<u32, DbError>;
}
