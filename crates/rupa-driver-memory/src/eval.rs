//! Expression evaluation with SQL semantics: three-valued logic (`NULL` is
//! "unknown"), Postgres' NULL ordering, and Postgres-compatible LIKE and casts.

use std::cmp::Ordering;

use rupa_core::ir::{BinOp, ColumnRef, ExprNode, PathSeg, UnOp};
use rupa_core::{SqlType, Value};

use crate::MemoryError;

/// Resolves column references against the row being evaluated.
pub(crate) trait Scope {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError>;
}

fn null_bool() -> Value {
    Value::Null(SqlType::Bool)
}

/// `Some(b)` for a boolean, `None` for NULL.
fn truth(v: &Value) -> Result<Option<bool>, MemoryError> {
    match v {
        Value::Bool(b) => Ok(Some(*b)),
        Value::Null(_) => Ok(None),
        other => Err(MemoryError::Type(format!(
            "expected a boolean, got {other:?}"
        ))),
    }
}

fn from_truth(t: Option<bool>) -> Value {
    t.map_or_else(null_bool, Value::Bool)
}

/// Whether a filter keeps the row: only TRUE does; FALSE and NULL drop it.
pub(crate) fn keeps(scope: &dyn Scope, filter: Option<&ExprNode>) -> Result<bool, MemoryError> {
    match filter {
        None => Ok(true),
        Some(f) => Ok(truth(&eval(scope, f)?)? == Some(true)),
    }
}

pub(crate) fn eval(scope: &dyn Scope, node: &ExprNode) -> Result<Value, MemoryError> {
    Ok(match node {
        ExprNode::Column(c) => scope.column(*c)?,
        ExprNode::Param(v) => v.clone(),
        ExprNode::Unary(UnOp::Not, e) => from_truth(truth(&eval(scope, e)?)?.map(|b| !b)),
        ExprNode::Unary(UnOp::IsNull, e) => Value::Bool(eval(scope, e)?.is_null()),
        ExprNode::Unary(UnOp::IsNotNull, e) => Value::Bool(!eval(scope, e)?.is_null()),
        ExprNode::Binary(BinOp::And, l, r) => {
            // FALSE wins over NULL in either position.
            match (truth(&eval(scope, l)?)?, truth(&eval(scope, r)?)?) {
                (Some(false), _) | (_, Some(false)) => Value::Bool(false),
                (Some(true), Some(true)) => Value::Bool(true),
                _ => null_bool(),
            }
        }
        ExprNode::Binary(BinOp::Or, l, r) => {
            match (truth(&eval(scope, l)?)?, truth(&eval(scope, r)?)?) {
                (Some(true), _) | (_, Some(true)) => Value::Bool(true),
                (Some(false), Some(false)) => Value::Bool(false),
                _ => null_bool(),
            }
        }
        ExprNode::Binary(BinOp::Like, l, r) => match (eval(scope, l)?, eval(scope, r)?) {
            (Value::Null(_), _) | (_, Value::Null(_)) => null_bool(),
            (Value::Text(s), Value::Text(p)) => Value::Bool(like(&s, &p)?),
            (a, b) => return Err(MemoryError::Type(format!("LIKE on {a:?} and {b:?}"))),
        },
        ExprNode::Binary(op, l, r) => {
            let (a, b) = (eval(scope, l)?, eval(scope, r)?);
            if a.is_null() || b.is_null() {
                return Ok(null_bool());
            }
            let ord = compare(&a, &b)?;
            Value::Bool(match op {
                BinOp::Eq => ord == Ordering::Equal,
                BinOp::Ne => ord != Ordering::Equal,
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Le => ord != Ordering::Greater,
                BinOp::Gt => ord == Ordering::Greater,
                BinOp::Ge => ord != Ordering::Less,
                BinOp::And | BinOp::Or | BinOp::Like => unreachable!("handled above"),
            })
        }
        ExprNode::In {
            expr,
            list,
            negated,
        } => {
            // x IN (a, b) = x = a OR x = b, with the usual NULL rules.
            let x = eval(scope, expr)?;
            let mut result = Some(false);
            for item in list {
                let v = eval(scope, item)?;
                let eq = if x.is_null() || v.is_null() {
                    None
                } else {
                    Some(compare(&x, &v)? == Ordering::Equal)
                };
                result = match (result, eq) {
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                };
            }
            from_truth(if *negated { result.map(|b| !b) } else { result })
        }
        ExprNode::JsonPath {
            column,
            path,
            as_text,
        } => json_path(scope.column(*column)?, path, *as_text)?,
        ExprNode::Cast(e, ty) => cast(eval(scope, e)?, *ty)?,
        ExprNode::Dsl(call) => {
            let eval_fn = call.def.eval.ok_or(MemoryError::Unsupported(format!(
                "DSL function `{}` has no memory-backend `eval`",
                call.def.name
            )))?;
            let args = call
                .args
                .iter()
                .map(|a| eval(scope, a))
                .collect::<Result<Vec<_>, _>>()?;
            eval_fn(&args)?
        }
        ExprNode::RawOp(op, ..) => {
            return Err(MemoryError::Unsupported(format!("raw SQL operator `{op}`")));
        }
        ExprNode::Call(name, _) => {
            return Err(MemoryError::Unsupported(format!(
                "raw SQL function `{name}`"
            )));
        }
    })
}

enum Num {
    Int(i128),
    Float(f64),
}

fn num(v: &Value) -> Option<Num> {
    Some(match v {
        Value::I16(n) => Num::Int(i128::from(*n)),
        Value::I32(n) => Num::Int(i128::from(*n)),
        Value::I64(n) => Num::Int(i128::from(*n)),
        Value::F32(n) => Num::Float(f64::from(*n)),
        Value::F64(n) => Num::Float(*n),
        _ => return None,
    })
}

/// Total order between two non-null values of compatible types.
pub(crate) fn compare(a: &Value, b: &Value) -> Result<Ordering, MemoryError> {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return Ok(match (x, y) {
            (Num::Int(x), Num::Int(y)) => x.cmp(&y),
            (x, y) => {
                let f = |n| match n {
                    Num::Int(i) => i as f64,
                    Num::Float(f) => f,
                };
                // Postgres orders NaN above all other values; total_cmp does too.
                f(x).total_cmp(&f(y))
            }
        });
    }
    Ok(match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        // Byte order, i.e. the "C" collation; see the crate docs.
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Bytes(x), Value::Bytes(y)) => x.cmp(y),
        (Value::Uuid(x), Value::Uuid(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::Time(x), Value::Time(y)) => x.cmp(y),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.cmp(y),
        (Value::TimestampTz(x), Value::TimestampTz(y)) => x.cmp(y),
        (Value::Json(x), Value::Json(y)) if x == y => Ordering::Equal,
        (Value::Json(_), Value::Json(_)) => {
            return Err(MemoryError::Unsupported("ordering of JSON values".into()));
        }
        (a, b) => {
            return Err(MemoryError::Type(format!(
                "cannot compare {a:?} with {b:?}"
            )));
        }
    })
}

/// Sort order for ORDER BY: NULLs compare greater than everything, which
/// gives Postgres' defaults (NULLS LAST ascending, NULLS FIRST descending).
pub(crate) fn sort_cmp(a: &Value, b: &Value) -> Result<Ordering, MemoryError> {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ok(Ordering::Equal),
        (true, false) => Ok(Ordering::Greater),
        (false, true) => Ok(Ordering::Less),
        (false, false) => compare(a, b),
    }
}

/// Postgres LIKE: `%` any run, `_` one character, `\` escapes the next one.
fn like(s: &str, pattern: &str) -> Result<bool, MemoryError> {
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
            '\\' => Tok::Lit(chars.next().ok_or_else(|| {
                MemoryError::Type("LIKE pattern must not end with escape character".into())
            })?),
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

/// `->` / `->>`: a missing key or index, or a path through a non-container,
/// is SQL NULL. With `->>`, JSON `null` is SQL NULL too.
fn json_path(value: Value, path: &[PathSeg], as_text: bool) -> Result<Value, MemoryError> {
    let null = Value::Null(if as_text {
        SqlType::Text
    } else {
        SqlType::Json
    });
    let mut cur = match value {
        Value::Json(j) => j,
        Value::Null(_) => return Ok(null),
        other => {
            return Err(MemoryError::Type(format!(
                "JSON path on non-JSON value {other:?}"
            )));
        }
    };
    for seg in path {
        let next = match (seg, &mut cur) {
            (PathSeg::Key(k), serde_json::Value::Object(m)) => m.remove(*k),
            (PathSeg::Index(i), serde_json::Value::Array(a)) => {
                let i = *i as usize;
                (i < a.len()).then(|| a.swap_remove(i))
            }
            _ => None,
        };
        match next {
            Some(v) => cur = v,
            None => return Ok(null),
        }
    }
    Ok(match (as_text, cur) {
        (false, j) => Value::Json(j),
        (true, serde_json::Value::Null) => null,
        (true, serde_json::Value::String(s)) => Value::Text(s),
        // Numbers and booleans print the same in serde_json and Postgres.
        // Objects and arrays do not (jsonb normalizes key order and spacing).
        (true, j @ (serde_json::Value::Number(_) | serde_json::Value::Bool(_))) => {
            Value::Text(j.to_string())
        }
        (true, _) => {
            return Err(MemoryError::Unsupported(
                "->> on a JSON object or array".into(),
            ));
        }
    })
}

fn cast(v: Value, ty: SqlType) -> Result<Value, MemoryError> {
    let fail = |v: &Value| MemoryError::Type(format!("cannot cast {v:?} to {ty:?}"));
    if v.is_null() {
        return Ok(Value::Null(ty));
    }
    let int = |v: &Value| -> Option<i128> {
        match num(v)? {
            Num::Int(i) => Some(i),
            Num::Float(_) => None,
        }
    };
    let text = match &v {
        Value::Text(s) => Some(s.trim()),
        _ => None,
    };
    let out = match ty {
        SqlType::Bool => match (&v, text) {
            (Value::Bool(b), _) => Some(Value::Bool(*b)),
            (_, Some(t)) => match t.to_ascii_lowercase().as_str() {
                "t" | "true" | "y" | "yes" | "on" | "1" => Some(Value::Bool(true)),
                "f" | "false" | "n" | "no" | "off" | "0" => Some(Value::Bool(false)),
                _ => None,
            },
            _ => None,
        },
        SqlType::I16 => text
            .and_then(|t| t.parse().ok())
            .or_else(|| int(&v)?.try_into().ok())
            .map(Value::I16),
        SqlType::I32 => text
            .and_then(|t| t.parse().ok())
            .or_else(|| int(&v)?.try_into().ok())
            .map(Value::I32),
        SqlType::I64 => text
            .and_then(|t| t.parse().ok())
            .or_else(|| int(&v)?.try_into().ok())
            .map(Value::I64),
        SqlType::F64 => text
            .and_then(|t| t.parse().ok())
            .or(match num(&v) {
                Some(Num::Int(i)) => Some(i as f64),
                Some(Num::Float(f)) => Some(f),
                None => None,
            })
            .map(Value::F64),
        SqlType::Text => match &v {
            Value::Text(s) => Some(Value::Text(s.clone())),
            Value::Bool(b) => Some(Value::Text(b.to_string())),
            other => int(other).map(|i| Value::Text(i.to_string())),
        },
        SqlType::Json => match (&v, text) {
            (Value::Json(j), _) => Some(Value::Json(j.clone())),
            (_, Some(t)) => serde_json::from_str(t).ok().map(Value::Json),
            _ => None,
        },
        SqlType::Uuid => text.and_then(|t| t.parse().ok()).map(Value::Uuid),
        _ => return Err(MemoryError::Unsupported(format!("cast to {ty:?}"))),
    };
    out.ok_or_else(|| fail(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_matches_postgres_semantics() {
        let cases = [
            ("abc", "abc", true),
            ("abc", "a%", true),
            ("abc", "%c", true),
            ("abc", "a_c", true),
            ("abc", "a_", false),
            ("abc", "%", true),
            ("", "%", true),
            ("", "_", false),
            ("a%c", "a\\%c", true),
            ("abc", "a\\%c", false),
            ("aXbXc", "%b%c", true),
            ("mississippi", "%iss%ppi", true),
            ("ABC", "abc", false),
        ];
        for (s, p, expect) in cases {
            assert_eq!(like(s, p).unwrap(), expect, "{s:?} LIKE {p:?}");
        }
        assert!(like("a", "a\\").is_err());
    }
}
