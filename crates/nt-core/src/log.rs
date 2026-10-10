//! `app.log`: one line per event, appended, never rewritten.
//!
//! Line format: `<UTC time> <event> <key=value> ...`, where the time is RFC
//! 3339 with milliseconds and a `Z`, and a value that holds whitespace, `"`,
//! or `=` is written as a JSON string so every line splits on spaces.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const PRIVATE_FILE_MODE: u32 = 0o600;

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
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since_epoch.as_secs();
    let (year, month, day) = civil_from_days(seconds / SECONDS_PER_DAY);
    let of_day = seconds % SECONDS_PER_DAY;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
        since_epoch.subsec_millis(),
    )
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
