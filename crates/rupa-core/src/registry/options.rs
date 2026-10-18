//! Untyped options of third-party drivers.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::config::{DriverConfig, parse_duration};
use crate::error::ConnectError;

/// Reads the untyped options of a third-party driver
/// ([`DriverConfig::Other`]), so that
/// keys the driver doesn't know are reported instead of silently ignored.
///
/// ```ignore
/// let mut opts = DriverOptions::new(&config.driver);
/// let busy = opts.duration("busy_timeout")?.unwrap_or(Duration::from_secs(5));
/// opts.finish()?; // errors on any key not read above
/// ```
#[derive(Debug)]
pub struct DriverOptions<'a> {
    remaining: BTreeMap<&'a str, &'a str>,
}

impl<'a> DriverOptions<'a> {
    /// Starts reading `driver`'s options; there are none unless it is
    /// [`DriverConfig::Other`].
    pub fn new(driver: &'a DriverConfig) -> Self {
        let remaining = match driver {
            DriverConfig::Other { options, .. } => options
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            _ => BTreeMap::new(),
        };
        Self { remaining }
    }

    /// Takes a string option.
    pub fn string(&mut self, key: &str) -> Option<&'a str> {
        self.remaining.remove(key)
    }

    /// Takes a duration option (see [`parse_duration`]).
    pub fn duration(&mut self, key: &str) -> Result<Option<Duration>, ConnectError> {
        self.parse(key, parse_duration)
    }

    /// Takes a boolean option (`true`/`false`/`on`/`off`/`1`/`0`).
    pub fn bool(&mut self, key: &str) -> Result<Option<bool>, ConnectError> {
        self.parse(key, |v| match v.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "1" | "yes" => Ok(true),
            "false" | "off" | "0" | "no" => Ok(false),
            other => Err(format!("expected a boolean, got `{other}`")),
        })
    }

    /// Takes an option and parses it with `parse`.
    pub fn parse<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, ConnectError> {
        self.string(key)
            .map(|v| {
                parse(v).map_err(|message| ConnectError::InvalidOption {
                    key: key.to_owned(),
                    message,
                })
            })
            .transpose()
    }

    /// Fails on the first option nobody took.
    pub fn finish(self) -> Result<(), ConnectError> {
        match self.remaining.into_keys().next() {
            None => Ok(()),
            Some(key) => Err(ConnectError::InvalidOption {
                key: key.to_owned(),
                message: "unknown option for this driver".into(),
            }),
        }
    }
}
