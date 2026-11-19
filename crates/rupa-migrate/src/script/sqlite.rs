use super::{ScriptError, block_comment, line_comment, quoted};

/// SQLite statements are authorized natively, so only the lexical grammar is checked.
pub(super) fn validate(sql: &str) -> Result<(), ScriptError> {
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
