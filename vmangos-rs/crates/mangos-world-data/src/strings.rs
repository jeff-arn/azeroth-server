//! `mangos_string` loading (`ObjectMgr::LoadMangosStrings`,
//! `ObjectMgr.cpp`, table form): entry + `content_default` +
//! `content_loc1..8`. Boot refuses to continue when the table
//! is empty (">> Loaded 0 mangos strings ... Cannot continue.").
//!
//! Deviation (recorded): the C++ stores locale texts at dynamic
//! indices assigned from the DBC locales present on disk
//! (`GetOrNewIndexForLocale`). Phase 3 keeps the eight locale
//! columns in their raw slots (1..=8 matching `LocaleConstant`)
//! and falls back to the default text when a locale slot is
//! empty — observably identical while only the enUS DBC set is
//! installed.

use std::collections::HashMap;

use mysql::prelude::Queryable;

use mangos_db::DbError;

/// `MIN_MANGOS_STRING_ID` (`ObjectMgr.h:161`); entry 0 is
/// reserved and ignored.
pub const MIN_MANGOS_STRING_ID: i32 = 1;
/// `MAX_MANGOS_STRING_ID` (`ObjectMgr.h:162`); the accepted
/// range is `[MIN, MAX)`.
pub const MAX_MANGOS_STRING_ID: i32 = 2_000_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MangosString {
    pub content_default: String,
    /// `content_loc1..=content_loc8`, `None` when empty.
    pub locales: [Option<String>; 8],
}

impl MangosString {
    /// Text for a `LocaleConstant` (0 = enUS default), falling
    /// back to the default when the locale slot is empty.
    #[must_use]
    pub fn text(&self, locale: u8) -> &str {
        if locale == 0 {
            return &self.content_default;
        }
        self.locales
            .get(usize::from(locale - 1))
            .and_then(|slot| slot.as_deref())
            .unwrap_or(&self.content_default)
    }
}

/// The loaded `mangos_string` table.
#[derive(Clone, Debug, Default)]
pub struct MangosStrings {
    entries: HashMap<i32, MangosString>,
}

impl MangosStrings {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, entry: i32) -> Option<&MangosString> {
        self.entries.get(&entry)
    }
}

/// One raw row: entry, default, loc1..=loc8 (NULL → `None`,
/// rendered "" by `Field::GetCppString`).
type MangosStringRow = (
    i32,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Row mapping with the C++ ignore rules: entry 0 reserved,
/// out-of-range ignored, first row for an entry wins (a later
/// row with content already present is ignored).
#[must_use]
pub fn map_mangos_string_rows(rows: Vec<MangosStringRow>) -> MangosStrings {
    let mut entries: HashMap<i32, MangosString> = HashMap::new();
    for row in rows {
        let entry = row.0;
        if entry == 0 || !(MIN_MANGOS_STRING_ID..MAX_MANGOS_STRING_ID).contains(&entry) {
            continue;
        }
        if entries.contains_key(&entry) {
            continue;
        }
        let loc = |s: Option<String>| s.filter(|v| !v.is_empty());
        entries.insert(
            entry,
            MangosString {
                content_default: row.1.unwrap_or_default(),
                locales: [
                    loc(row.2),
                    loc(row.3),
                    loc(row.4),
                    loc(row.5),
                    loc(row.6),
                    loc(row.7),
                    loc(row.8),
                    loc(row.9),
                ],
            },
        );
    }
    MangosStrings { entries }
}

/// Load `mangos_string` from the world DB. An empty result is
/// returned as an empty store; the caller applies the C++
/// cannot-continue rule.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_mangos_strings(conn: &mut mysql::Conn) -> Result<MangosStrings, DbError> {
    let rows: Vec<MangosStringRow> = conn.query(
        "SELECT `entry`, `content_default`, `content_loc1`, `content_loc2`, \
                `content_loc3`, `content_loc4`, `content_loc5`, `content_loc6`, \
                `content_loc7`, `content_loc8` FROM `mangos_string`",
    )?;
    Ok(map_mangos_string_rows(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(entry: i32, default: &str, loc2: Option<&str>) -> MangosStringRow {
        (
            entry,
            Some(default.to_string()),
            None,
            loc2.map(str::to_string),
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn ignore_rules_and_locale_fallback() {
        let strings = map_mangos_string_rows(vec![
            row(0, "reserved", None),
            row(-5, "out of range", None),
            row(2_000_000_000, "at max (excluded)", None),
            row(7, "hello", Some("bonjour")),
            row(7, "duplicate", None),
            row(9, "world", None),
        ]);
        assert_eq!(strings.len(), 2);
        let seven = strings.get(7).expect("entry 7");
        assert_eq!(seven.text(0), "hello");
        assert_eq!(seven.text(2), "bonjour"); // content_loc2
        assert_eq!(seven.text(3), "hello"); // empty slot -> default
        assert_eq!(seven.text(9), "hello"); // out-of-range locale -> default
        assert!(strings.get(0).is_none());
    }
}
