use super::{ScriptError, line_comment, quoted};

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

fn block_comment(bytes: &[u8], start: usize) -> Result<usize, ScriptError> {
    let close = bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .ok_or(ScriptError::Unterminated(start))?;

    Ok(start + 2 + close + 2)
}
