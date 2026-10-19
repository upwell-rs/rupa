use std::fmt;

/// The explicit grammar and ordering convention for one migration stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VersionScheme {
    /// Canonical positive decimal `u64` identifiers.
    Numbered,
    /// Fourteen-digit Gregorian UTC timestamps, to the second.
    UtcTimestamp,
}

/// A validated migration version. Compare only within one configured scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    scheme: VersionScheme,
    value: u64,
}

/// A version is not canonical or is outside the scheme's range.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {scheme} migration version: {input:?}")]
pub struct VersionError {
    scheme: &'static str,
    input: String,
}

impl VersionScheme {
    /// The stable history representation of this scheme.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Numbered => "numbered",
            Self::UtcTimestamp => "utc-timestamp",
        }
    }

    /// Parses and validates one canonical identifier.
    pub fn parse(self, input: &str) -> Result<Version, VersionError> {
        let invalid = || VersionError {
            scheme: self.as_str(),
            input: input.into(),
        };
        let value = if !input.is_empty() && input.bytes().all(|b| b.is_ascii_digit()) {
            input.parse::<u64>().map_err(|_| invalid())?
        } else {
            return Err(invalid());
        };

        match self {
            Self::Numbered if value != 0 && !input.starts_with('0') => Ok(Version {
                scheme: self,
                value,
            }),
            Self::UtcTimestamp if valid_timestamp(input) => Ok(Version {
                scheme: self,
                value,
            }),
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            VersionScheme::Numbered => write!(f, "{}", self.value),
            VersionScheme::UtcTimestamp => write!(f, "{:014}", self.value),
        }
    }
}

fn valid_timestamp(input: &str) -> bool {
    if input.len() != 14 {
        return false;
    }

    let component = |start, end| {
        input[start..end]
            .parse::<u32>()
            .expect("validated ASCII digits")
    };
    let year = component(0, 4);
    let month = component(4, 6);
    let day = component(6, 8);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };

    year > 0
        && (1..=days).contains(&day)
        && component(8, 10) < 24
        && component(10, 12) < 60
        && component(12, 14) < 60
}

#[cfg(test)]
mod tests;
