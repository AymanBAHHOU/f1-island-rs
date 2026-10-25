//! A small log of what live timing reports about a session's status (Started, Finished,
//! Inactive, ...), with times, one JSON object per line. A few lines per session; it
//! exists to learn the real sequence. (Port of `src/statuslog.js`.)

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// A log file past this size is kept as `<file>.old` and restarted.
pub const MAX_BYTES: u64 = 256 * 1024;

/// What goes in one line, besides the time.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub event: String,
    pub session: Option<String>,
    /// Qualifying segment (1, 2 or 3), if the feed says.
    pub part: Option<u8>,
    pub status: Option<String>,
    pub clock: Option<String>,
}

/// One line of JSON with the time first: `{"t":"2026-10-24T21:00:00.000Z","event":"status",...}`.
pub fn format_line(entry: &Entry, now: DateTime<Utc>) -> String {
    // A struct that borrows (`&'a`) the entry instead of copying it. `#[serde(flatten)]`
    // puts the entry's fields right after `t` in the same object.
    #[derive(Serialize)]
    struct Line<'a> {
        t: String,
        #[serde(flatten)]
        entry: &'a Entry,
    }
    let line = Line {
        t: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        entry,
    };
    serde_json::to_string(&line).expect("an Entry always serialises")
}

/// Appends one line, first moving a too-big file to `<file>.old`.
///
/// Returns an `io::Result` instead of crashing: logging must not disturb the app, so
/// the caller can simply ignore a failure (`let _ = append_entry(...)`).
pub fn append_entry(
    file: &Path,
    entry: &Entry,
    now: DateTime<Utc>,
    max_bytes: u64,
) -> io::Result<()> {
    if fs::metadata(file).is_ok_and(|m| m.len() > max_bytes) {
        let mut old = file.as_os_str().to_owned();
        old.push(".old");
        fs::rename(file, old)?;
    }
    let mut out = OpenOptions::new().create(true).append(true).open(file)?;
    writeln!(out, "{}", format_line(entry, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, h, m, 0).unwrap()
    }

    fn status(text: &str) -> Entry {
        Entry {
            event: "status".into(),
            status: Some(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_line_is_json_with_the_time_first() {
        let line = format_line(&status("Started"), at(21, 0));
        assert!(
            line.starts_with(r#"{"t":"2026-10-24T21:00:00.000Z","event":"status""#),
            "{line}"
        );

        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["status"], "Started");
        assert!(value["session"].is_null()); // missing values are logged as null
    }

    #[test]
    fn lines_are_appended_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("live-status.log");
        append_entry(
            &file,
            &Entry {
                event: "connect".into(),
                ..Default::default()
            },
            at(20, 45),
            MAX_BYTES,
        )
        .unwrap();
        append_entry(&file, &status("Started"), at(21, 0), MAX_BYTES).unwrap();

        let text = fs::read_to_string(&file).unwrap();
        let events: Vec<String> = text
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["event"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(events, ["connect", "status"]);
    }

    #[test]
    fn a_big_file_is_kept_as_old_and_restarted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("live-status.log");
        append_entry(&file, &status("Started"), at(21, 0), 50).unwrap(); // bigger than 50 bytes
        append_entry(&file, &status("Finished"), at(22, 0), 50).unwrap();

        assert!(fs::read_to_string(&file).unwrap().contains("Finished"));
        assert!(fs::read_to_string(dir.path().join("live-status.log.old"))
            .unwrap()
            .contains("Started"));
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("missing").join("x.log");
        assert!(append_entry(&file, &status("Started"), at(21, 0), MAX_BYTES).is_err());
    }
}
