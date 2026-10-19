mod postgres;
mod sqlite;

use std::fmt;

use rupa_core::dialect::DialectId;

/// Why a migration script was refused before any of it was dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScriptError {
    /// A quote, comment or dollar-quoted body is never closed.
    #[error("unterminated SQL quote or comment at byte {0}")]
    Unterminated(usize),
    /// A statement the migration surface refuses, starting at byte `at`.
    #[error("{rule} at byte {at}")]
    Forbidden {
        /// The refused statement class.
        rule: ScriptRule,
        /// Byte offset of the offending statement or token.
        at: usize,
    },
    /// The session's dialect has no migration script grammar yet.
    #[error("migration scripts are not supported on dialect {0:?}")]
    Dialect(DialectId),
}

/// A statement class migration scripts and raw queries must not contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptRule {
    /// `BEGIN`, `COMMIT`, `ROLLBACK`, savepoints and prepared transactions.
    TransactionControl,
    /// Session-level `SET`, `set_config`, `RESET`, `DISCARD`, `LISTEN` and `UNLISTEN`.
    SessionState,
    /// Client commands such as psql backslash commands and `COPY` from stdin.
    ClientCommand,
    /// Advisory lock functions, which could release the runner's exclusion.
    AdvisoryLock,
    /// Names reserved for migration metadata (`_rupa_` prefix).
    ReservedName,
    /// `BEGIN ATOMIC` bodies, whose statement boundaries the lexer cannot know.
    AtomicBody,
}

impl fmt::Display for ScriptRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TransactionControl => "transaction control is owned by the migration runner",
            Self::SessionState => "session state changes are not allowed in migrations",
            Self::ClientCommand => "client commands are not SQL",
            Self::AdvisoryLock => "advisory locks are owned by the migration runner",
            Self::ReservedName => "names starting with _rupa_ are reserved",
            Self::AtomicBody => "BEGIN ATOMIC bodies are unsupported; use a dollar-quoted body",
        })
    }
}

/// Lexically prevalidates a whole script with the session dialect's grammar.
///
/// Dialects without an implemented grammar fail explicitly instead of being
/// scanned with rules that could misread their escapes or quoting. Drivers
/// without a native authorizer also validate rendered raw queries with it.
pub fn validate_script(sql: &str, dialect: DialectId) -> Result<(), ScriptError> {
    match dialect {
        DialectId::Sqlite => sqlite::validate(sql),
        DialectId::Postgres => postgres::validate(sql),
        _ => Err(ScriptError::Dialect(dialect)),
    }
}

fn quoted(bytes: &[u8], start: usize, end: u8, doubled: bool) -> Result<usize, ScriptError> {
    let mut index = start + 1;

    while index < bytes.len() {
        if bytes[index] != end {
            index += 1;

            continue;
        }

        if doubled && bytes.get(index + 1) == Some(&end) {
            index += 2;

            continue;
        }

        return Ok(index + 1);
    }

    Err(ScriptError::Unterminated(start))
}

fn line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset)
}

#[cfg(test)]
mod tests;
