//! Dates and times as text, for databases without date types (SQLite).
//!
//! Every driver that stores them as text uses these formats, so a database
//! written by one driver reads the same through another. The formats sort
//! as text in value order: fixed width, always six fractional digits, and
//! `Z` for UTC instants.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

const DATE: &str = "%Y-%m-%d";
const TIME: &str = "%H:%M:%S%.6f";
const TIMESTAMP: &str = "%Y-%m-%dT%H:%M:%S%.6f";
const TIMESTAMP_TZ: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";

/// Timestamp forms accepted when reading, besides the one written.
const TIMESTAMP_INPUTS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%d %H:%M:%S",
];

/// `YYYY-MM-DD`.
pub fn format_date(d: &NaiveDate) -> String {
    d.format(DATE).to_string()
}

/// `HH:MM:SS.ffffff`.
pub fn format_time(t: &NaiveTime) -> String {
    t.format(TIME).to_string()
}

/// `YYYY-MM-DDTHH:MM:SS.ffffff`.
pub fn format_timestamp(t: &NaiveDateTime) -> String {
    t.format(TIMESTAMP).to_string()
}

/// `YYYY-MM-DDTHH:MM:SS.ffffffZ`, in UTC.
pub fn format_timestamp_tz(t: &DateTime<Utc>) -> String {
    t.format(TIMESTAMP_TZ).to_string()
}

/// A date written by [`format_date`].
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, DATE).ok()
}

/// A time of day, with or without fractional seconds.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M:%S%.f").ok()
}

/// A timestamp: `T` or space separated, fractional seconds optional, a
/// trailing `Z` ignored.
pub fn parse_timestamp(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim_end_matches('Z');

    TIMESTAMP_INPUTS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
}

/// An instant: RFC 3339, a space-separated timestamp with an offset, or a
/// timestamp without one (taken as UTC).
pub fn parse_timestamp_tz(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%:z"))
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| parse_timestamp(s).map(|t| t.and_utc()))
}
