use rupa_core::DialectId;
use sha2::{Digest, Sha256};

use crate::Policy;

/// Domain tag and version of the canonical target identity bytes.
const IDENTITY_FORMAT: &str = "rupa-migration-target-v1";

/// Prefix of MySQL named migration locks.
const LOCK_PREFIX: &str = "rupa:";

/// A database engine whose migration targets share one exclusion protocol,
/// whichever driver connects to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    /// PostgreSQL, through any driver.
    Postgres,
    /// MySQL, through any driver.
    MySql,
    /// SQLite, through any driver.
    Sqlite,
}

impl Engine {
    /// The stable name used in canonical identity bytes.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::MySql => "mysql",
            Self::Sqlite => "sqlite",
        }
    }

    /// The SQL dialect the engine speaks.
    pub const fn dialect(self) -> DialectId {
        match self {
            Self::Postgres => DialectId::Postgres,
            Self::MySql => DialectId::MySql,
            Self::Sqlite => DialectId::Sqlite,
        }
    }

    /// The execution policies the engine's sessions honour. MySQL commits DDL
    /// implicitly, so it runs nontransactional bodies only; SQLite has no
    /// autocommit body protocol, so it runs transactional ones only.
    pub const fn policies(self) -> &'static [Policy] {
        match self {
            Self::Postgres => &[Policy::TransactionalRequired, Policy::NonTransactional],
            Self::MySql => &[Policy::NonTransactional],
            Self::Sqlite => &[Policy::TransactionalRequired],
        }
    }
}

/// The canonical identity of a migration target.
///
/// Built from the engine, the selected database and the owned namespace only,
/// so it is independent of driver, credentials, URL spelling and stream: every
/// runner sharing an owned namespace derives the same exclusion identity.
/// Digest collisions can only over-serialize runners, never bypass exclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target<'a> {
    /// The engine the target runs on.
    pub engine: Engine,
    /// The selected database, as the server names it.
    pub database: &'a str,
    /// The namespace owning the migration metadata, such as a PostgreSQL schema.
    pub namespace: &'a str,
}

impl Target<'_> {
    /// SHA-256 of the versioned, length-prefixed UTF-8 identity components.
    pub fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();

        for component in [
            IDENTITY_FORMAT,
            self.engine.as_str(),
            self.database,
            self.namespace,
        ] {
            hash.update((component.len() as u64).to_be_bytes());
            hash.update(component.as_bytes());
        }

        hash.finalize().into()
    }

    /// The PostgreSQL advisory lock key: the digest's first eight bytes as a
    /// signed big-endian integer.
    pub fn advisory_key(&self) -> i64 {
        let digest = self.digest();
        let mut key = [0; 8];

        key.copy_from_slice(&digest[..8]);

        i64::from_be_bytes(key)
    }

    /// The MySQL named lock: `rupa:` and the digest's first 28 bytes as 56
    /// lowercase hex digits, 61 characters within MySQL's 64-character limit.
    pub fn lock_name(&self) -> String {
        let mut name = String::with_capacity(LOCK_PREFIX.len() + 56);

        name.push_str(LOCK_PREFIX);

        for byte in &self.digest()[..28] {
            name.push_str(&format!("{byte:02x}"));
        }

        name
    }
}

#[cfg(test)]
mod tests;
