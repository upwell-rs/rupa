use rupa_core::dialect::DialectId;

use crate::ValidationError;

/// Lexically prevalidates a whole script with the session dialect's grammar.
///
/// Dialects without an implemented grammar fail explicitly instead of being
/// scanned with rules that could misread their escapes or quoting.
pub(crate) fn validate(sql: &str, dialect: DialectId) -> Result<(), ValidationError> {
    match dialect {
        DialectId::Sqlite => validate_sqlite(sql),
        _ => Err(ValidationError::ScriptDialect(dialect)),
    }
}

fn validate_sqlite(sql: &str) -> Result<(), ValidationError> {
    let bytes = sql.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let start = index;

        index = match bytes[index] {
            b'\'' | b'"' | b'`' => quoted(bytes, index, bytes[index], true)?,
            b'[' => quoted(bytes, index, b']', false)?,
            b'-' if bytes.get(index + 1) == Some(&b'-') => line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => block_comment(bytes, start)?,
            _ => index + 1,
        };
    }

    Ok(())
}

fn quoted(bytes: &[u8], start: usize, end: u8, doubled: bool) -> Result<usize, ValidationError> {
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

    Err(ValidationError::Sql(start))
}

fn line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset)
}

fn block_comment(bytes: &[u8], start: usize) -> Result<usize, ValidationError> {
    let close = bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .ok_or(ValidationError::Sql(start))?;

    Ok(start + 2 + close + 2)
}

#[cfg(test)]
mod tests;
