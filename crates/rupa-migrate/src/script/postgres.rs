use super::{ScriptError, ScriptRule, line_comment, quoted};

/// PostgreSQL has no client-side authorizer, so statement classes the runner
/// owns are refused lexically, assuming `standard_conforming_strings`.
/// Dollar-quoted bodies are opaque: what they execute is the author's trust
/// boundary, guarded by the session's transaction and lock checks.
pub(super) fn validate(sql: &str) -> Result<(), ScriptError> {
    let bytes = sql.as_bytes();
    let mut statement = Statement::default();
    let mut index = 0;

    while index < bytes.len() {
        let start = index;

        index = match bytes[index] {
            b'\'' => quoted(bytes, index, b'\'', true)?,
            b'"' => {
                let end = quoted(bytes, index, b'"', true)?;

                statement.word(&sql[start + 1..end - 1], start, true)?;

                end
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => nested_comment(bytes, start)?,
            b'$' => dollar_quoted(bytes, index)?,
            b'\\' => return Err(forbidden(ScriptRule::ClientCommand, start)),
            b';' => {
                statement = Statement::default();

                index + 1
            }
            byte if word_start(byte) => {
                let end = word_end(bytes, index);

                if matches!(&sql[start..end], "E" | "e") && bytes.get(end) == Some(&b'\'') {
                    escaped(bytes, start, end)?
                } else {
                    statement.word(&sql[start..end], start, false)?;

                    end
                }
            }
            _ => index + 1,
        };
    }

    Ok(())
}

/// The leading keyword of a statement whose meaning depends on later words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Leading {
    #[default]
    Other,
    Set,
    Prepare,
    Copy,
}

/// Classification state for the statement being scanned.
#[derive(Debug, Default)]
struct Statement {
    leading: Leading,
    start: usize,
    words: usize,
    after_begin: bool,
}

impl Statement {
    fn word(&mut self, word: &str, at: usize, quoted: bool) -> Result<(), ScriptError> {
        let lower = word.to_ascii_lowercase();
        let keyword = if quoted { "" } else { lower.as_str() };

        if lower.starts_with("_rupa_") {
            return Err(forbidden(ScriptRule::ReservedName, at));
        }

        if lower == "set_config" {
            return Err(forbidden(ScriptRule::SessionState, at));
        }

        if lower.starts_with("pg_advisory") || lower.starts_with("pg_try_advisory") {
            return Err(forbidden(ScriptRule::AdvisoryLock, at));
        }

        if self.after_begin && keyword == "atomic" {
            return Err(forbidden(ScriptRule::AtomicBody, at));
        }

        self.after_begin = keyword == "begin";

        if self.words == 0 {
            self.start = at;
            self.leading = leading(keyword, at)?;
        } else {
            self.follow(keyword)?;
        }

        self.words += 1;

        Ok(())
    }

    fn follow(&self, keyword: &str) -> Result<(), ScriptError> {
        let rule = match self.leading {
            Leading::Set
                if self.words == 1
                    && !matches!(keyword, "local" | "constraints" | "transaction") =>
            {
                Some(ScriptRule::SessionState)
            }
            Leading::Prepare if self.words == 1 && keyword == "transaction" => {
                Some(ScriptRule::TransactionControl)
            }
            Leading::Copy if matches!(keyword, "stdin" | "stdout") => {
                Some(ScriptRule::ClientCommand)
            }
            _ => None,
        };

        match rule {
            Some(rule) => Err(forbidden(rule, self.start)),
            None => Ok(()),
        }
    }
}

fn leading(keyword: &str, at: usize) -> Result<Leading, ScriptError> {
    let rule = match keyword {
        "begin" | "start" | "commit" | "end" | "rollback" | "abort" | "savepoint" | "release" => {
            ScriptRule::TransactionControl
        }
        "reset" | "discard" | "listen" | "unlisten" => ScriptRule::SessionState,
        "set" => return Ok(Leading::Set),
        "prepare" => return Ok(Leading::Prepare),
        "copy" => return Ok(Leading::Copy),
        _ => return Ok(Leading::Other),
    };

    Err(forbidden(rule, at))
}

fn forbidden(rule: ScriptRule, at: usize) -> ScriptError {
    ScriptError::Forbidden { rule, at }
}

fn word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn word_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| !(word_start(*byte) || byte.is_ascii_digit() || *byte == b'$'))
        .map_or(bytes.len(), |offset| start + offset)
}

/// An `E'...'` string, where a backslash escapes the next byte.
fn escaped(bytes: &[u8], start: usize, quote: usize) -> Result<usize, ScriptError> {
    let mut index = quote + 1;

    while index < bytes.len() {
        index = match bytes[index] {
            b'\\' => index + 2,
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index + 2,
            b'\'' => return Ok(index + 1),
            _ => index + 1,
        };
    }

    Err(ScriptError::Unterminated(start))
}

fn nested_comment(bytes: &[u8], start: usize) -> Result<usize, ScriptError> {
    let mut depth = 0usize;
    let mut index = start;

    while index + 1 < bytes.len() {
        match &bytes[index..index + 2] {
            b"/*" => {
                depth += 1;
                index += 2;
            }
            b"*/" => {
                depth -= 1;
                index += 2;

                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => index += 1,
        }
    }

    Err(ScriptError::Unterminated(start))
}

/// A `$tag$...$tag$` body, or a lone `$` such as a `$1` parameter. A `$`
/// inside a word never reaches this, since words may contain `$`.
fn dollar_quoted(bytes: &[u8], start: usize) -> Result<usize, ScriptError> {
    let tag_end = match bytes.get(start + 1) {
        Some(b'$') => start + 1,
        Some(byte) if word_start(*byte) => {
            let end = bytes[start + 1..]
                .iter()
                .position(|byte| !(word_start(*byte) || byte.is_ascii_digit()))
                .map_or(bytes.len(), |offset| start + 1 + offset);

            if bytes.get(end) != Some(&b'$') {
                return Ok(start + 1);
            }

            end
        }
        _ => return Ok(start + 1),
    };
    let tag = &bytes[start..=tag_end];
    let body = tag_end + 1;

    bytes[body..]
        .windows(tag.len())
        .position(|window| window == tag)
        .map(|offset| body + offset + tag.len())
        .ok_or(ScriptError::Unterminated(start))
}
