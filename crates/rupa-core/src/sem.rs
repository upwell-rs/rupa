//! SQL semantics shared by evaluators: the memory backend and the `eval`
//! implementations of DSL functions. Matching Postgres where observable.

#[cfg(test)]
mod tests;

/// Postgres `LIKE`: `%` any run, `_` one character, `\` escapes the next one.
/// Case-sensitive; callers lowercase both sides for `ILIKE`.
///
/// Iterative wildcard matching with backtracking to the last `%`.
pub fn like(s: &str, pattern: &str) -> Result<bool, String> {
    enum Tok {
        Any,
        One,
        Lit(char),
    }
    let mut toks = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        toks.push(match c {
            '%' => Tok::Any,
            '_' => Tok::One,
            '\\' => Tok::Lit(
                chars
                    .next()
                    .ok_or_else(|| "LIKE pattern must not end with escape character".to_string())?,
            ),
            c => Tok::Lit(c),
        });
    }

    let s: Vec<char> = s.chars().collect();
    let (mut si, mut pi) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match toks.get(pi) {
            Some(Tok::Any) => {
                star = Some((pi, si));
                pi += 1;
            }
            Some(Tok::One) => {
                si += 1;
                pi += 1;
            }
            Some(Tok::Lit(c)) if *c == s[si] => {
                si += 1;
                pi += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    pi = sp + 1;
                    si = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return Ok(false),
            },
        }
    }

    Ok(toks[pi..].iter().all(|t| matches!(t, Tok::Any)))
}

/// Postgres `jsonb @>`: whether `outer` contains `inner`.
///
/// - Objects: every key of `inner` is in `outer`, with a contained value.
/// - Arrays: every element of `inner` is contained in some element of
///   `outer` (order and duplicates ignored). As in Postgres, an array also
///   contains a bare primitive that is one of its top-level elements.
/// - Scalars: equal.
pub fn json_contains(outer: &serde_json::Value, inner: &serde_json::Value) -> bool {
    use serde_json::Value as J;
    match (outer, inner) {
        (J::Object(o), J::Object(i)) => i
            .iter()
            .all(|(k, iv)| o.get(k).is_some_and(|ov| json_contains(ov, iv))),
        (J::Array(o), J::Array(i)) => i.iter().all(|iv| o.iter().any(|ov| json_contains(ov, iv))),
        (J::Array(o), prim) if !prim.is_object() && !prim.is_array() => {
            o.iter().any(|ov| ov == prim)
        }
        (J::Number(a), J::Number(b)) => a.as_f64() == b.as_f64(),
        (a, b) => a == b,
    }
}
