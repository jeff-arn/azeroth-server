//! Byte-compatible port of the C++ `Config` (`vmangos/src/shared/Config/Config.cpp`).
//!
//! The parser is a hand-written state machine; its quirks are observable behaviour and
//! are preserved deliberately:
//! - Only the space character (`0x20`) counts as whitespace. Tabs are name/value bytes.
//! - `#` or `[` while reading a name discards the whole line.
//! - Quotes in values are effectively decorative: each `"` is dropped and reading
//!   continues (the C++ `quoteFound` flag resets per character), so a `#` always ends
//!   the value and text after a closing quote keeps appending.
//! - Lines with an empty name or value are discarded; duplicate keys keep the FIRST
//!   occurrence.
//! - Input is consumed in `fgets` chunks of at most 1023 bytes, and each chunk is
//!   parsed as its own "line" — a >1023-byte line therefore parses twice.
//! - Lookup is case-sensitive. `GetBoolDefault` accepts `true/TRUE/yes/YES/1`.
//!
//! Deviation: C++ `GetIntDefault`/`GetFloatDefault` call `std::stoi`/`std::stof`, which
//! throw (uncaught) on malformed values. Rust returns the default instead of crashing.

use std::collections::HashMap;
use std::path::Path;

/// Errors returned by [`Config::load_from_file`].
#[derive(Debug)]
pub enum ConfigError {
    /// The config file could not be read.
    Io(std::io::Error),
    /// The config file parsed to zero settings (C++ `Reload()` returns false).
    Empty,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "reading config file: {e}"),
            ConfigError::Empty => write!(f, "config file contains no settings"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(e) => Some(e),
            ConfigError::Empty => None,
        }
    }
}

const FGETS_CHUNK: usize = 1023;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Stage {
    FindName,
    ReadName,
    FindValue,
    ReadValue,
}

/// One parsed line: `Some((name, value))` or `None` when discarded.
fn process_line(line: &[u8]) -> Option<(String, String)> {
    let mut stage = Stage::FindName;
    let mut name: Vec<u8> = Vec::new();
    let mut value: Vec<u8> = Vec::new();

    for &ch in line {
        if ch == 0 || ch == b'\n' || ch == b'\r' {
            break;
        }
        let mut stop = false;
        let quote_found = false; // C++ resets this per character; '#' therefore always stops.
        match stage {
            Stage::FindName | Stage::ReadName => match ch {
                b'#' | b'[' => return None,
                b' ' => {}
                b'=' => {
                    if stage == Stage::FindName || name.is_empty() {
                        return None;
                    }
                    stage = Stage::FindValue;
                }
                _ => {
                    name.push(ch);
                    stage = Stage::ReadName;
                }
            },
            Stage::FindValue | Stage::ReadValue => match ch {
                b'#' => {
                    if !quote_found {
                        stop = true;
                    }
                }
                b'"' => {
                    stage = Stage::ReadValue;
                }
                b' ' if stage == Stage::FindValue => {}
                _ => {
                    value.push(ch);
                    stage = Stage::ReadValue;
                }
            },
        }
        if stop {
            break;
        }
    }

    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((
        String::from_utf8_lossy(&name).into_owned(),
        String::from_utf8_lossy(&value).into_owned(),
    ))
}

/// Split input the way `fgets(buffer, 1024, file)` does: chunks end after a `\n` or at
/// 1023 bytes, whichever comes first.
fn fgets_chunks(input: &[u8]) -> Vec<&[u8]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < input.len() {
        let limit = (start + FGETS_CHUNK).min(input.len());
        let end = input[start..limit]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(limit, |pos| start + pos + 1);
        chunks.push(&input[start..end]);
        start = end;
    }
    chunks
}

/// In-memory config, mirroring the C++ `Config` map (first occurrence wins).
#[derive(Debug, Default)]
pub struct Config {
    map: HashMap<String, String>,
}

impl Config {
    /// Parse raw config bytes (C++ `Reload()` body).
    #[must_use]
    pub fn parse(input: &[u8]) -> Config {
        let mut cfg = Config::default();
        for chunk in fgets_chunks(input) {
            if let Some((name, value)) = process_line(chunk) {
                cfg.map.entry(name).or_insert(value);
            }
        }
        cfg
    }

    /// Load and parse a config file (C++ `LoadFromFile`). Fails when unreadable or when
    /// the file yields no settings.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Io`] when the file cannot be read and
    /// [`ConfigError::Empty`] when it contains no settings.
    pub fn load_from_file(path: &Path) -> Result<Config, ConfigError> {
        let bytes = std::fs::read(path).map_err(ConfigError::Io)?;
        let cfg = Config::parse(&bytes);
        if cfg.map.is_empty() {
            return Err(ConfigError::Empty);
        }
        Ok(cfg)
    }

    /// Number of settings held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether no settings are held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// C++ `IsSet`.
    #[must_use]
    pub fn is_set(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// Raw value lookup (C++ `GetValueHelper`).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.map.get(name).map(String::as_str)
    }

    /// C++ `GetStringDefault`.
    #[must_use]
    pub fn string_default(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or(default).to_string()
    }

    /// C++ `GetBoolDefault`: true only for `true`, `TRUE`, `yes`, `YES`, `1`.
    #[must_use]
    pub fn bool_default(&self, name: &str, default: bool) -> bool {
        match self.get(name) {
            None => default,
            Some(v) => matches!(v, "true" | "TRUE" | "yes" | "YES" | "1"),
        }
    }

    /// C++ `GetIntDefault` (`std::stoi`; malformed values yield the default here
    /// instead of the C++ uncaught-throw crash).
    #[must_use]
    pub fn int_default(&self, name: &str, default: i32) -> i32 {
        match self.get(name) {
            None => default,
            Some(v) => parse_stoi_prefix(v).unwrap_or(default),
        }
    }

    /// C++ `GetFloatDefault` (`std::stof`; see [`Config::int_default`]).
    #[must_use]
    pub fn float_default(&self, name: &str, default: f32) -> f32 {
        match self.get(name) {
            None => default,
            Some(v) => v.trim().parse::<f32>().unwrap_or(default),
        }
    }
}

/// `std::stoi` accepts a numeric prefix and ignores the tail; mirror that.
fn parse_stoi_prefix(v: &str) -> Option<i32> {
    let bytes = v.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    v[..end].parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Config {
        Config::parse(text.as_bytes())
    }

    #[test]
    fn basic_assignments_and_spacing() {
        let cfg = parse("Port = 3724\nBindIP=\"0.0.0.0\"\n");
        assert_eq!(cfg.get("Port"), Some("3724"));
        assert_eq!(cfg.get("BindIP"), Some("0.0.0.0"));
    }

    #[test]
    fn comment_ends_value_and_comment_lines_discarded() {
        let cfg = parse("# full comment\nKey = val # trailing\n[section]\nOther = x\n");
        assert_eq!(cfg.get("Key"), Some("val "));
        assert_eq!(cfg.get("Other"), Some("x"));
        assert_eq!(cfg.len(), 2);
    }

    #[test]
    fn quote_quirks_match_cpp() {
        // '#' inside quotes still stops (quoteFound resets per char); text after a
        // closing quote keeps appending; quotes themselves are dropped.
        let cfg = parse("A = \"a#b\"\nB = \"c d\" e\n");
        assert_eq!(cfg.get("A"), Some("a"));
        assert_eq!(cfg.get("B"), Some("c d e"));
    }

    #[test]
    fn tabs_are_not_whitespace() {
        let cfg = parse("Key\t= x\n");
        assert_eq!(cfg.get("Key\t"), Some("x"));
        assert_eq!(cfg.get("Key"), None);
    }

    #[test]
    fn duplicates_keep_first_and_empties_discarded() {
        let cfg = parse("K = 1\nK = 2\nEmpty =\n= noval\nSpaced =   \n");
        assert_eq!(cfg.get("K"), Some("1"));
        assert_eq!(cfg.get("Empty"), None);
        assert_eq!(cfg.get("Spaced"), None);
        assert_eq!(cfg.len(), 1);
    }

    #[test]
    fn long_line_is_chunked_like_fgets() {
        // A 1050-char value-bearing line: first chunk stores truncated value; the tail
        // chunk parses as its own line.
        let mut text = String::from("Key = ");
        text.push_str(&"a".repeat(1017)); // 6 + 1017 = 1023 bytes in chunk 1
        text.push_str("Tail = x\n");
        let cfg = Config::parse(text.as_bytes());
        assert_eq!(cfg.get("Key").map(str::len), Some(1017));
        assert_eq!(cfg.get("Tail"), Some("x"));
    }

    #[test]
    fn typed_accessors() {
        let cfg = parse("I = 42junk\nNeg = -7\nF = 1.5\nT = YES\nF2 = off\n");
        assert_eq!(cfg.int_default("I", 0), 42);
        assert_eq!(cfg.int_default("Neg", 0), -7);
        assert_eq!(cfg.int_default("Missing", 3), 3);
        assert!((cfg.float_default("F", 0.0) - 1.5).abs() < f32::EPSILON);
        assert!(cfg.bool_default("T", false));
        assert!(!cfg.bool_default("F2", true));
        assert_eq!(cfg.string_default("Missing", "d"), "d");
        assert!(cfg.is_set("I"));
    }
}
