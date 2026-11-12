use std::ops::Range;

use super::{
    Ranges, ScriptError, ScriptRule, block_comment, forbidden, line_comment, quoted, word_end,
    word_start,
};

/// MySQL has no client-side authorizer, so statement classes the runner owns
/// are refused lexically, assuming `NO_BACKSLASH_ESCAPES`, which the migration
/// session sets. Executable comments and `BEGIN … END` compound statements
/// are refused instead of guessed at, so every top-level semicolon ends a
/// statement.
///
/// Returns the byte range of every statement holding more than whitespace and
/// comments, without its terminating semicolon.
pub(super) fn statements(sql: &str) -> Result<Vec<Range<usize>>, ScriptError> {
    let bytes = sql.as_bytes();
    let mut ranges = Ranges::default();
    let mut statement = Statement::default();
    let mut index = 0;

    while index < bytes.len() {
        let start = index;
        let comment = line_comment_start(bytes, index) || bytes[index..].starts_with(b"/*");

        ranges.token(comment || bytes[index] == b';' || bytes[index].is_ascii_whitespace());
        index = match bytes[index] {
            b'\'' | b'"' => quoted(bytes, index, bytes[index], true)?,
            b'`' => {
                let end = quoted(bytes, index, b'`', true)?;

                statement.word(&sql[start + 1..end - 1], start, true)?;

                end
            }
            _ if line_comment_start(bytes, index) => line_comment(bytes, index),
            b'/' if bytes[index..].starts_with(b"/*") => {
                if bytes[index + 2..].starts_with(b"!") || bytes[index + 2..].starts_with(b"M!") {
                    return Err(forbidden(ScriptRule::ExecutableComment, start));
                }

                block_comment(bytes, start)?
            }
            b'\\' => return Err(forbidden(ScriptRule::ClientCommand, start)),
            b';' => {
                ranges.end(start);
                statement = Statement::default();

                index + 1
            }
            byte if word_start(byte) => {
                let end = word_end(bytes, index);

                statement.word(&sql[start..end], start, false)?;

                end
            }
            _ => index + 1,
        };
    }

    Ok(ranges.finish(bytes.len()))
}

/// `#` always starts a line comment; `--` only when whitespace, a control
/// character or the end of input follows, otherwise it is two minus signs.
fn line_comment_start(bytes: &[u8], index: usize) -> bool {
    match &bytes[index..] {
        [b'#', ..] | [b'-', b'-'] => true,
        [b'-', b'-', next, ..] => next.is_ascii_whitespace() || next.is_ascii_control(),
        _ => false,
    }
}

/// The leading keyword of a statement whose meaning depends on later words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Leading {
    #[default]
    Other,
    Load,
}

/// Classification state for the statement being scanned.
#[derive(Debug, Default)]
struct Statement {
    leading: Leading,
    start: usize,
    words: usize,
}

impl Statement {
    fn word(&mut self, word: &str, at: usize, quoted: bool) -> Result<(), ScriptError> {
        let lower = word.to_ascii_lowercase();
        let keyword = if quoted { "" } else { lower.as_str() };

        if lower.starts_with("_rupa_") {
            return Err(forbidden(ScriptRule::ReservedName, at));
        }

        if matches!(
            lower.as_str(),
            "get_lock" | "release_lock" | "release_all_locks"
        ) {
            return Err(forbidden(ScriptRule::AdvisoryLock, at));
        }

        if self.words == 0 {
            self.start = at;
            self.leading = leading(keyword, at)?;
        } else if keyword == "begin" {
            return Err(forbidden(ScriptRule::CompoundStatement, at));
        } else if self.leading == Leading::Load && keyword == "local" {
            return Err(forbidden(ScriptRule::ClientCommand, self.start));
        }

        self.words += 1;

        Ok(())
    }
}

fn leading(keyword: &str, at: usize) -> Result<Leading, ScriptError> {
    let rule = match keyword {
        "begin" | "start" | "commit" | "rollback" | "savepoint" | "release" | "xa" | "lock"
        | "unlock" => ScriptRule::TransactionControl,
        "set" | "use" | "reset" => ScriptRule::SessionState,
        "delimiter" | "source" => ScriptRule::ClientCommand,
        "load" => return Ok(Leading::Load),
        _ => return Ok(Leading::Other),
    };

    Err(forbidden(rule, at))
}
