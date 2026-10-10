//! The logs, appended and never rewritten.
//!
//! `app.log` line format: `<UTC time> <event> <key=value> ...`, where the
//! time is RFC 3339 with milliseconds and a `Z`, and a value that holds
//! whitespace, `"`, or `=` is written as a JSON string so every line splits
//! on spaces.
//!
//! Each agent log holds one JSON object per line:
//! `{"ts":"<UTC time>","dir":"in"|"out"|"err","line":"<raw line>"}`.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::home::PRIVATE_FILE_MODE;

#[derive(Debug)]
pub struct AppLog {
    file: File,
}

impl AppLog {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(PRIVATE_FILE_MODE)
            .open(path)?;
        Ok(Self { file })
    }

    /// Appends one line. `event` may hold more than one word, such as
    /// `quit done` or `metric cold_start_ms=12`.
    pub fn write(&mut self, event: &str, fields: &[(&str, &str)]) -> io::Result<()> {
        let line = format_line(SystemTime::now(), event, fields);
        self.file.write_all(line.as_bytes())
    }
}

/// Which way a raw agent line went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Written to the agent's stdin.
    In,
    /// Read from its stdout.
    Out,
    /// Read from its stderr.
    Err,
}

impl Direction {
    const fn field(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Err => "err",
        }
    }
}

/// `logs/agents/<target>-<UTC start time>-<pid>.jsonl`: every raw line in
/// and out of one agent process.
#[derive(Debug)]
pub struct AgentLog {
    file: File,
    path: PathBuf,
}

impl AgentLog {
    pub fn open(dir: &Path, target: &str, started: SystemTime, pid: u32) -> io::Result<Self> {
        let path = dir.join(format!("{target}-{}-{pid}.jsonl", compact_utc(started)));
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&path)?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&mut self, direction: Direction, line: &str) -> io::Result<()> {
        let mut record = serde_json::json!({
            "ts": rfc3339_millis(SystemTime::now()),
            "dir": direction.field(),
            "line": line,
        })
        .to_string();
        record.push('\n');
        self.file.write_all(record.as_bytes())
    }
}

fn format_line(time: SystemTime, event: &str, fields: &[(&str, &str)]) -> String {
    let mut line = format!("{} {event}", rfc3339_millis(time));
    for (key, value) in fields {
        let _ = write!(line, " {key}={}", log_value(value));
    }
    line.push('\n');
    line
}

fn log_value(value: &str) -> String {
    let needs_quotes = value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '=');
    if needs_quotes {
        json_string(value)
    } else {
        value.to_owned()
    }
}

fn json_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(quoted, "\\u{:04x}", u32::from(c));
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

const SECONDS_PER_DAY: u64 = 86_400;

/// UTC time as `2026-10-10T17:00:47.123Z`.
pub fn rfc3339_millis(time: SystemTime) -> String {
    let utc = Utc::from(time);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        utc.year, utc.month, utc.day, utc.hour, utc.minute, utc.second, utc.millis
    )
}

/// UTC time as `20261010T170047Z`, for file names.
pub fn compact_utc(time: SystemTime) -> String {
    let utc = Utc::from(time);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        utc.year, utc.month, utc.day, utc.hour, utc.minute, utc.second
    )
}

/// The calendar parts of a UTC time from 1970 on.
#[derive(Clone, Copy, Debug)]
struct Utc {
    year: u64,
    month: u64,
    day: u64,
    hour: u64,
    minute: u64,
    second: u64,
    millis: u32,
}

impl From<SystemTime> for Utc {
    fn from(time: SystemTime) -> Self {
        let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let seconds = since_epoch.as_secs();
        let (year, month, day) = civil_from_days(seconds / SECONDS_PER_DAY);
        let of_day = seconds % SECONDS_PER_DAY;
        Self {
            year,
            month,
            day,
            hour: of_day / 3600,
            minute: of_day % 3600 / 60,
            second: of_day % 60,
            millis: since_epoch.subsec_millis(),
        }
    }
}

/// The proleptic Gregorian date of a count of days since 1970-01-01, by
/// Howard Hinnant's `civil_from_days`
/// (<https://howardhinnant.github.io/date_algorithms.html#civil_from_days>),
/// restricted to dates from 1970 on.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
