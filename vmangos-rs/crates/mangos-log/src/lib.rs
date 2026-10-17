//! Minimal slice of the C++ `Log` (`vmangos/src/shared/Log.cpp`) needed by `realmd`:
//! the basic log file with the C++ line format preserved:
//! `YYYY-MM-DD HH:MM:SS ` (local time) + `ERROR: ` for error level + message.
//! The C++ overwrites the basic log on open (`OpenLogFile(..., overwriteOnOpen=true)`,
//! or timestamped name when `LogFile.TimeStamp=1`); that behaviour is kept.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// C++ `LogLevel` values (higher = more verbose).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// `LOG_LVL_ERROR` (0)
    Error = 0,
    /// `LOG_LVL_MINIMAL` (1)
    Minimal = 1,
    /// `LOG_LVL_BASIC` (2)
    Basic = 2,
    /// `LOG_LVL_DETAIL` (3)
    Detail = 3,
    /// `LOG_LVL_DEBUG` (4)
    Debug = 4,
}

fn now_local() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

fn timestamp_prefix(t: time::OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} ",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

fn file_timestamp_suffix(t: time::OffsetDateTime) -> String {
    format!(
        "_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// Format one log line exactly as `LOG_TO_FILE_HELPER` does.
fn format_line(at: time::OffsetDateTime, level: Level, message: &str) -> String {
    let mut line = timestamp_prefix(at);
    if level == Level::Error {
        line.push_str("ERROR: ");
    }
    line.push_str(message);
    line.push('\n');
    line
}

/// The basic log: one file, one verbosity threshold (`LogFile.Level`, C++ default 2).
pub struct Log {
    file: Mutex<Option<File>>,
    file_level: Level,
}

impl Log {
    /// Open the basic log file. Mirrors `Log::OpenLogFile`: the file is (over)written,
    /// and with `timestamped` the `_YYYY-MM-DD_HH-MM-SS` suffix is inserted before the
    /// last `.`. An empty file name disables file logging (C++ returns nullptr).
    ///
    /// # Errors
    ///
    /// Returns the OS error when the file cannot be created.
    pub fn open(
        logs_dir: &Path,
        file_name: &str,
        timestamped: bool,
        file_level: Level,
    ) -> std::io::Result<Log> {
        if file_name.is_empty() {
            return Ok(Log {
                file: Mutex::new(None),
                file_level,
            });
        }
        let mut name = file_name.to_string();
        if timestamped {
            let suffix = file_timestamp_suffix(now_local());
            if let Some(dot) = name.rfind('.') {
                name.insert_str(dot, &suffix);
            } else {
                name.push_str(&suffix);
            }
        }
        let path: PathBuf = logs_dir.join(name);
        let file = File::create(path)?;
        Ok(Log {
            file: Mutex::new(Some(file)),
            file_level,
        })
    }

    /// C++ `sLog.Out(LOG_BASIC, level, ...)`: dropped above the configured file level.
    ///
    /// # Panics
    ///
    /// Panics only if the internal log mutex was poisoned by an earlier panic.
    pub fn out(&self, level: Level, message: &str) {
        if self.file_level < level {
            return;
        }
        let line = format_line(now_local(), level, message);
        let mut guard = self.file.lock().expect("log mutex poisoned");
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed() -> time::OffsetDateTime {
        time::macros::datetime!(2026-10-16 2:20:59 UTC)
    }

    #[test]
    fn line_format_matches_cpp() {
        assert_eq!(
            format_line(fixed(), Level::Basic, "Core revision: abc"),
            "2026-10-16 02:20:59 Core revision: abc\n"
        );
        assert_eq!(
            format_line(fixed(), Level::Error, "boom"),
            "2026-10-16 02:20:59 ERROR: boom\n"
        );
        assert_eq!(file_timestamp_suffix(fixed()), "_2026-10-16_02-20-59");
    }

    #[test]
    fn level_filter() {
        let dir = std::env::temp_dir().join(format!("mangos-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let log = Log::open(&dir, "Test.log", false, Level::Basic).expect("open");
        log.out(Level::Error, "kept");
        log.out(Level::Debug, "dropped");
        drop(log);
        let contents = std::fs::read_to_string(dir.join("Test.log")).expect("read");
        assert!(contents.contains("ERROR: kept"));
        assert!(!contents.contains("dropped"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
