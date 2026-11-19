mod mysql;
mod postgres;
mod sqlite;

use std::fmt;
use std::ops::Range;

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
    /// `BEGIN`, `COMMIT`, `ROLLBACK`, savepoints, prepared and XA transactions
    /// and MySQL `LOCK`/`UNLOCK TABLES`.
    TransactionControl,
    /// Session-level `SET`, `set_config`, `RESET`, `DISCARD`, `LISTEN` and
    /// `UNLISTEN`; on MySQL every `SET` and `USE`.
    SessionState,
    /// Client commands such as psql backslash commands, `COPY` from stdin and
    /// the MySQL client's `DELIMITER`, `SOURCE` and `LOAD DATA LOCAL`.
    ClientCommand,
    /// Advisory or named lock functions, which could release the runner's exclusion.
    AdvisoryLock,
    /// Names reserved for migration metadata (`_rupa_` prefix).
    ReservedName,
    /// `BEGIN ATOMIC` bodies, whose statement boundaries the lexer cannot know.
    AtomicBody,
    /// MySQL `BEGIN … END` compound statements, whose boundaries the lexer cannot know.
    CompoundStatement,
    /// MySQL executable comments (`/*! … */`), which run SQL the grammar cannot check.
    ExecutableComment,
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
            Self::CompoundStatement => {
                "BEGIN ... END compound statements are unsupported; use single-statement routines"
            }
            Self::ExecutableComment => "executable comments are not allowed in migrations",
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
        DialectId::Postgres => postgres::statements(sql).map(drop),
        DialectId::MySql => mysql::statements(sql).map(drop),
        _ => Err(ScriptError::Dialect(dialect)),
    }
}

/// Validates a whole script, then returns the source byte range of each of
/// its statements, without terminating semicolons, comment-only or empty ones.
///
/// Drivers use it to dispatch statements one at a time where a single
/// multi-statement request would run in one implicit transaction. Only
/// dialects whose grammar knows every statement boundary can be split: SQLite
/// trigger bodies, for one, contain unquoted semicolons.
pub fn split_script(sql: &str, dialect: DialectId) -> Result<Vec<Range<usize>>, ScriptError> {
    match dialect {
        DialectId::Postgres => postgres::statements(sql),
        DialectId::MySql => mysql::statements(sql),
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

fn forbidden(rule: ScriptRule, at: usize) -> ScriptError {
    ScriptError::Forbidden { rule, at }
}

/// Whether `byte` starts an unquoted identifier or keyword.
fn word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// The end of the identifier or keyword starting at `start`; `$` may follow
/// the first byte in both PostgreSQL and MySQL.
fn word_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| !(word_start(*byte) || byte.is_ascii_digit() || *byte == b'$'))
        .map_or(bytes.len(), |offset| start + offset)
}

/// Statement byte ranges collected while a grammar scans a script.
#[derive(Debug, Default)]
struct Ranges {
    ranges: Vec<Range<usize>>,
    begin: usize,
    significant: bool,
}

impl Ranges {
    /// Records a token; whitespace, comments and terminators are inert.
    fn token(&mut self, inert: bool) {
        self.significant |= !inert;
    }

    /// Ends the statement at the terminator at `at`, keeping it if it held a token.
    fn end(&mut self, at: usize) {
        if self.significant {
            self.ranges.push(self.begin..at);
        }

        self.begin = at + 1;
        self.significant = false;
    }

    /// The ranges, including an unterminated final statement ending at `len`.
    fn finish(mut self, len: usize) -> Vec<Range<usize>> {
        if self.significant {
            self.ranges.push(self.begin..len);
        }

        self.ranges
    }
}

/// A non-nesting `/* … */` comment.
fn block_comment(bytes: &[u8], start: usize) -> Result<usize, ScriptError> {
    let close = bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .ok_or(ScriptError::Unterminated(start))?;

    Ok(start + 2 + close + 2)
}

fn line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset)
}

#[cfg(test)]
mod tests;
