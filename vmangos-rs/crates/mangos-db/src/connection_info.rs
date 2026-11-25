//! `*DatabaseInfo` connection strings: `host;port;user;password;db`
//! with exactly four semicolons (`Master::StartDB` rejects any
//! other shape as "Incorrectly formatted database connection
//! string"). Promoted from the Rust realmd's `login_db_url` so
//! both daemons share one parser.

use std::fmt::Write as _;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionInfo {
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl ConnectionInfo {
    /// Parse a `host;port;user;password;db` string. `None` unless
    /// there are exactly four semicolons, like the C++ check.
    #[must_use]
    pub fn parse(dbstring: &str) -> Option<ConnectionInfo> {
        if dbstring.matches(';').count() != 4 {
            return None;
        }
        let parts: Vec<&str> = dbstring.split(';').collect();
        Some(ConnectionInfo {
            host: parts[0].to_string(),
            port: parts[1].to_string(),
            user: parts[2].to_string(),
            password: parts[3].to_string(),
            database: parts[4].to_string(),
        })
    }

    /// `mysql://` URL for the pure-Rust driver, with the fields
    /// that may contain URL-reserved bytes percent-encoded.
    #[must_use]
    pub fn mysql_url(&self) -> String {
        format!(
            "mysql://{}:{}@{}:{}/{}",
            percent_encode(&self.user),
            percent_encode(&self.password),
            self.host,
            self.port,
            percent_encode(&self.database)
        )
    }

    /// The string as `Master::StartDB` logs it, password masked.
    #[must_use]
    pub fn masked(&self) -> String {
        format!(
            "{};{};{};*;{}",
            self.host, self.port, self.user, self.database
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_render() {
        let info = ConnectionInfo::parse("127.0.0.1;3306;mangos;mangos;realmd").expect("parse");
        assert_eq!(
            info.mysql_url(),
            "mysql://mangos:mangos@127.0.0.1:3306/realmd"
        );
        assert_eq!(info.masked(), "127.0.0.1;3306;mangos;*;realmd");
    }

    #[test]
    fn special_chars_are_encoded() {
        let info = ConnectionInfo::parse("db;3306;user;p@ss/w;db name").expect("parse");
        assert_eq!(
            info.mysql_url(),
            "mysql://user:p%40ss%2Fw@db:3306/db%20name"
        );
    }

    #[test]
    fn semicolon_count_is_strict() {
        assert!(ConnectionInfo::parse("a;b;c;d").is_none());
        assert!(ConnectionInfo::parse("a;b;c;d;e;f").is_none());
        assert!(ConnectionInfo::parse("").is_none());
        assert!(ConnectionInfo::parse(";;;;").is_some()); // empty fields parse; connect fails later
    }
}
