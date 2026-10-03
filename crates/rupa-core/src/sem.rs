//! SQL semantics shared by evaluators: the memory backend and the `eval`
//! implementations of DSL functions. Matching Postgres where observable.

/// Postgres `LIKE`: `%` any run, `_` one character, `\` escapes the next one.
/// Case-sensitive; callers lowercase both sides for `ILIKE`.
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
    // Iterative wildcard matching with backtracking to the last `%`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn like_matches_postgres_semantics() {
        let cases = [
            ("abc", "abc", true),
            ("abc", "a%", true),
            ("abc", "%c", true),
            ("abc", "a_c", true),
            ("abc", "a_", false),
            ("", "%", true),
            ("", "_", false),
            ("a%c", r"a\%c", true),
            ("abc", r"a\%c", false),
            ("mississippi", "%iss%ppi", true),
            ("ABC", "abc", false),
        ];
        for (s, p, expect) in cases {
            assert_eq!(like(s, p).unwrap(), expect, "{s:?} LIKE {p:?}");
        }
        assert!(like("a", "a\\").is_err());
    }

    #[test]
    fn json_containment_matches_postgres() {
        let doc = json!({"a": 1, "tags": ["x", "y"], "n": {"k": true, "z": null}});
        assert!(json_contains(&doc, &json!({})));
        assert!(json_contains(&doc, &json!({"a": 1})));
        assert!(json_contains(&doc, &json!({"a": 1.0})));
        assert!(json_contains(&doc, &json!({"tags": ["y"]})));
        assert!(json_contains(&doc, &json!({"n": {"k": true}})));
        assert!(json_contains(&doc, &json!({"n": {"z": null}})));
        assert!(!json_contains(&doc, &json!({"a": 2})));
        assert!(!json_contains(&doc, &json!({"tags": ["q"]})));
        assert!(!json_contains(&doc, &json!({"missing": null})));
        assert!(json_contains(&json!([1, 2, [3]]), &json!([[3], 1])));
        assert!(json_contains(&json!(["a", "b"]), &json!("a")));
        assert!(!json_contains(&json!({"a": 1}), &json!([])));
    }
}
