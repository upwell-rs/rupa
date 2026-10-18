//! Durations in configuration: a number of milliseconds, or `"500ms"`,
//! `"5s"`, `"2m"`, `"1h"`; `"off"` (or null) for none.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serializer, de};

// Durations
// ---------------------------------------------------------------------------

/// Parses a duration: a number of milliseconds, or a number with a unit
/// (`ms`, `s`, `m`, `h`), e.g. `"250ms"`, `"30s"`, `"1.5s"`, `"10m"`.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (number, unit) = s.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| format!("invalid duration `{s}`"))?;
    let seconds = match unit.trim() {
        "" | "ms" => number / 1000.0,
        "s" => number,
        "m" | "min" => number * 60.0,
        "h" => number * 3600.0,
        other => return Err(format!("unknown duration unit `{other}` in `{s}`")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|e| format!("invalid duration `{s}`: {e}"))
}

/// Formats a duration the way [`parse_duration`] reads it.
pub fn format_duration(d: Duration) -> String {
    match d.as_millis() {
        _ if !d.subsec_nanos().is_multiple_of(1_000_000) => format!("{}s", d.as_secs_f64()),
        ms if ms % 3_600_000 == 0 && ms > 0 => format!("{}h", ms / 3_600_000),
        ms if ms % 60_000 == 0 && ms > 0 => format!("{}m", ms / 60_000),
        ms if ms % 1000 == 0 && ms > 0 => format!("{}s", ms / 1000),
        ms => format!("{ms}ms"),
    }
}

/// Serializes `Option<Duration>` as `"30s"` or null (serde `with`).
pub(super) fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
    match d {
        Some(d) => s.serialize_str(&format_duration(*d)),
        None => s.serialize_none(),
    }
}

/// A duration as configured: milliseconds, or text.
#[derive(Deserialize)]
#[serde(untagged)]
enum Repr {
    Millis(u64),
    Text(String),
}

/// Deserializes `Option<Duration>` from `"30s"`, milliseconds, `"off"` or
/// null (serde `with`).
pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    Ok(match Option::<Repr>::deserialize(d)? {
        None => None,
        Some(Repr::Millis(ms)) => Some(Duration::from_millis(ms)),
        Some(Repr::Text(s)) if matches!(s.trim(), "off" | "none" | "never") => None,
        Some(Repr::Text(s)) => Some(parse_duration(&s).map_err(de::Error::custom)?),
    })
}
