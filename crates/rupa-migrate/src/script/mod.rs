use crate::ValidationError;

pub(crate) fn validate(sql: &str) -> Result<(), ValidationError> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        match bytes[index] {
            b'\'' | b'"' | b'`' | b'[' => {
                let end = if bytes[index] == b'[' {
                    b']'
                } else {
                    bytes[index]
                };
                let doubled = bytes[index] != b'[';
                index += 1;
                loop {
                    if index == bytes.len() {
                        return Err(ValidationError::Sql(start));
                    }

                    if bytes[index] == end {
                        index += 1;
                        if doubled && bytes.get(index) == Some(&end) {
                            index += 1;
                        } else {
                            break;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                    index += 1;
                }
                if index + 1 >= bytes.len() {
                    return Err(ValidationError::Sql(start));
                }

                index += 2;
            }
            _ => index += 1,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
