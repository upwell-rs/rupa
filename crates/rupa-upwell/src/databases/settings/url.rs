use std::fmt::Write as _;

use super::{DatabaseSettings, SettingsError};

/// A SQLite path naming a private in-memory database.
const MEMORY_PATH: &str = ":memory:";

/// The databases a driver connects to, which decide the URL's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    /// `postgres://user:password@host:port/name?params`.
    Postgres,
    /// `mysql://user:password@host:port/name?params`.
    MySql,
    /// `sqlite://path` or `sqlite::memory:`.
    Sqlite,
    /// `memory:`.
    Memory,
}

impl Family {
    /// The family of the rupa driver `driver`.
    pub(super) fn of(driver: &str) -> Option<Self> {
        let family = match driver {
            "postgres" | "postgres-sync" | "sqlx-postgres" => Self::Postgres,
            "mysql" | "sqlx-mysql" => Self::MySql,
            "sqlite" | "sqlx-sqlite" => Self::Sqlite,
            "memory" => Self::Memory,
            _ => return None,
        };

        Some(family)
    }
}

/// The URL `settings` describes for `family`, with `user` and `password`.
pub(super) fn build(
    family: Family,
    driver: &str,
    settings: &DatabaseSettings,
    user: Option<&str>,
    password: Option<&str>,
) -> Result<String, SettingsError> {
    let unused = |field| SettingsError::Unused {
        driver: driver.to_owned(),
        field,
    };
    let url = match family {
        Family::Memory => String::from("memory:"),
        Family::Sqlite => {
            let networked = [
                ("host", settings.host.is_some()),
                ("port", settings.port.is_some()),
                ("user", user.is_some()),
                ("password", password.is_some()),
                ("name", settings.name.is_some()),
            ];

            if let Some((field, _)) = networked.into_iter().find(|(_, set)| *set) {
                return Err(unused(field));
            }

            let mut url = match settings.path.as_deref() {
                Some(MEMORY_PATH) => String::from("sqlite::memory:"),
                Some(path) => format!("sqlite://{path}"),
                None => {
                    return Err(SettingsError::Missing {
                        driver: driver.to_owned(),
                        field: "path",
                    });
                }
            };

            query(&mut url, settings);

            url
        }
        Family::Postgres | Family::MySql => {
            if settings.path.is_some() {
                return Err(unused("path"));
            }

            networked(family, driver, settings, user, password)?
        }
    };

    Ok(url)
}

/// `scheme://user:password@host:port/name?params`, each part encoded.
/// Writing to a `String` cannot fail, so those results are ignored.
fn networked(
    family: Family,
    driver: &str,
    settings: &DatabaseSettings,
    user: Option<&str>,
    password: Option<&str>,
) -> Result<String, SettingsError> {
    let host = settings
        .host
        .as_deref()
        .ok_or_else(|| SettingsError::Missing {
            driver: driver.to_owned(),
            field: "host",
        })?;
    let scheme = match family {
        Family::MySql => "mysql",
        Family::Postgres | Family::Sqlite | Family::Memory => "postgres",
    };
    let mut url = format!("{scheme}://");

    if let Some(user) = user {
        url.push_str(&encode(user));

        if let Some(password) = password {
            url.push(':');
            url.push_str(&encode(password));
        }

        url.push('@');
    }

    if host.contains(':') && !host.starts_with('[') {
        let _ = write!(url, "[{host}]");
    } else {
        url.push_str(host);
    }

    if let Some(port) = settings.port {
        let _ = write!(url, ":{port}");
    }

    url.push('/');

    if let Some(name) = &settings.name {
        url.push_str(&encode(name));
    }

    query(&mut url, settings);

    Ok(url)
}

/// Appends `settings.params` to `url` as an encoded query.
fn query(url: &mut String, settings: &DatabaseSettings) {
    for (index, (key, value)) in settings.params.iter().enumerate() {
        url.push(if index == 0 { '?' } else { '&' });
        url.push_str(&encode(key));
        url.push('=');
        url.push_str(&encode(value));
    }
}

/// `url` with its credentials replaced by `user` and `password`.
pub(super) fn with_userinfo(
    url: &str,
    user: &str,
    password: Option<&str>,
) -> Result<String, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| String::from("it has no `scheme://` authority"))?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = match rest[..authority_end].rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    let credentials = match password {
        Some(password) => format!("{}:{}", encode(user), encode(password)),
        None => encode(user),
    };

    Ok(format!("{scheme}://{credentials}@{host}"))
}

/// `text` percent-encoded, keeping only RFC 3986 unreserved characters.
/// Writing to a `String` cannot fail, so that result is ignored.
fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());

    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }

    encoded
}
