//! `exploration_basexp` (`ObjectMgr::LoadExplorationBaseXP`,
//! `GetBaseXP`): per-level base XP for area discovery.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// `m_BaseXPMap`: level -> basexp (missing level = 0).
#[derive(Clone, Debug, Default)]
pub struct ExplorationBaseXp {
    by_level: HashMap<u32, u32>,
}

impl ExplorationBaseXp {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_level.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_level.is_empty()
    }

    /// `ObjectMgr::GetBaseXP`.
    #[must_use]
    pub fn base_xp(&self, level: u32) -> u32 {
        self.by_level.get(&level).copied().unwrap_or(0)
    }

    fn from_rows(rows: impl IntoIterator<Item = (u32, u32)>) -> Self {
        Self {
            by_level: rows.into_iter().collect(),
        }
    }
}

/// Loads all `exploration_basexp` rows.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_exploration_base_xp(conn: &mut mysql::Conn) -> Result<ExplorationBaseXp, DbError> {
    let rows: Vec<mysql::Row> =
        conn.exec("SELECT `level`, `basexp` FROM `exploration_basexp`", ())?;
    Ok(ExplorationBaseXp::from_rows(rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        (f.u32(0), f.u32(1))
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_xp_missing_level_is_zero() {
        let store = ExplorationBaseXp::from_rows([(20, 110)]);
        assert_eq!(store.base_xp(20), 110);
        assert_eq!(store.base_xp(21), 0);
        assert_eq!(store.len(), 1);
    }
}
