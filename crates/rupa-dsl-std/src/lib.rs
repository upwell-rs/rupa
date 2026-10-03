//! Built-in DSL functions, written with the same `#[dsl::function]` macro
//! available to users. Nothing here is special-cased in the grammar or core.
//!
//! | function | gating | Postgres | memory backend |
//! |---|---|---|---|
//! | [`ilike()`] | runtime (per dialect) | `a ILIKE b` | `eval` |
//! | [`lower()`], [`upper()`] | none (standard SQL) | `lower(a)` | `eval` |
//! | [`json_contains()`] | static: `JsonContainment` | `a @> b` (MySQL `JSON_CONTAINS`) | `eval` |
//! | [`json_has_key()`] | static: `JsonPath` | `jsonb_exists(a, k)` (MySQL `JSON_CONTAINS_PATH`, SQLite `json_type`) | `eval` |
//!
//! Every function takes and returns expressions; arguments that carry data
//! are bound parameters.

use rupa_core::ir::ExprNode;
use rupa_core::sem;
use rupa_core::{Dialect, DialectId, DslError, Expr, ExprOps, SqlType, Value};
use rupa_macros::function;

type Json = serde_json::Value;

fn text_args<'a>(name: &'static str, args: &'a [Value]) -> Result<Option<Vec<&'a str>>, DslError> {
    let mut out = Vec::with_capacity(args.len());
    for v in args {
        match v {
            Value::Text(s) => out.push(s.as_str()),
            Value::Null(_) => return Ok(None),
            other => {
                return Err(DslError::Other(format!(
                    "`{name}` expects text, got {other:?}"
                )));
            }
        }
    }
    Ok(Some(out))
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Case-insensitive `LIKE`. Runtime-checked: Postgres has `ILIKE`; MySQL and
/// SQLite get `LOWER(a) LIKE LOWER(b)`; other dialects are an error at render.
#[function(crate = ::rupa_core, eval = eval_ilike)]
pub fn ilike(
    dialect: &dyn Dialect,
    s: Expr<String>,
    pattern: Expr<String>,
) -> Result<Expr<bool>, DslError> {
    match dialect.id() {
        DialectId::Postgres => Ok(Expr::raw_op("ILIKE", s, pattern)),
        // A `LIKE` node (not a raw operator), so dialect-specific LIKE
        // handling such as SQLite's `ESCAPE` clause applies.
        DialectId::MySql | DialectId::Sqlite => {
            Ok(Expr::<String>::call("LOWER", [s]).like(Expr::<String>::call("LOWER", [pattern])))
        }
        other => Err(DslError::unsupported("ilike", other)),
    }
}

fn eval_ilike(args: &[Value]) -> Result<Value, DslError> {
    Ok(match text_args("ilike", args)? {
        Some(a) => Value::Bool(
            sem::like(&a[0].to_lowercase(), &a[1].to_lowercase()).map_err(DslError::Other)?,
        ),
        None => Value::Null(SqlType::Bool),
    })
}

/// `lower(s)`: standard SQL, available on every dialect.
#[function(crate = ::rupa_core, eval = eval_lower)]
pub fn lower(s: Expr<String>) -> Expr<String> {
    Expr::call("lower", [s])
}

fn eval_lower(args: &[Value]) -> Result<Value, DslError> {
    Ok(match text_args("lower", args)? {
        Some(a) => Value::Text(a[0].to_lowercase()),
        None => Value::Null(SqlType::Text),
    })
}

/// `upper(s)`: standard SQL, available on every dialect.
#[function(crate = ::rupa_core, eval = eval_upper)]
pub fn upper(s: Expr<String>) -> Expr<String> {
    Expr::call("upper", [s])
}

fn eval_upper(args: &[Value]) -> Result<Value, DslError> {
    Ok(match text_args("upper", args)? {
        Some(a) => Value::Text(a[0].to_uppercase()),
        None => Value::Null(SqlType::Text),
    })
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn json_args<'a>(name: &'static str, args: &'a [Value]) -> Result<Option<Vec<&'a Json>>, DslError> {
    let mut out = Vec::with_capacity(args.len());
    for v in args {
        match v {
            Value::Json(j) => out.push(j),
            Value::Null(_) => return Ok(None),
            other => {
                return Err(DslError::Other(format!(
                    "`{name}` expects JSON, got {other:?}"
                )));
            }
        }
    }
    Ok(Some(out))
}

/// Whether JSON `doc` contains `part` (Postgres `jsonb @>`). Statically gated:
/// using it in a repository over a dialect without `JsonContainment` does
/// not compile.
#[function(crate = ::rupa_core, requires = JsonContainment, eval = eval_json_contains)]
pub fn json_contains(
    dialect: &dyn Dialect,
    doc: Expr<Json>,
    part: Expr<Json>,
) -> Result<Expr<bool>, DslError> {
    match dialect.id() {
        DialectId::Postgres => Ok(Expr::raw_op("@>", doc, part)),
        DialectId::MySql => Ok(Expr::call("JSON_CONTAINS", [doc, part])),
        other => Err(DslError::unsupported("json_contains", other)),
    }
}

fn eval_json_contains(args: &[Value]) -> Result<Value, DslError> {
    Ok(match json_args("json_contains", args)? {
        Some(a) => Value::Bool(sem::json_contains(a[0], a[1])),
        None => Value::Null(SqlType::Bool),
    })
}

/// Whether JSON object `doc` has top-level key `key`. Statically gated on
/// `JsonPath`. On MySQL and SQLite the key goes into a JSON path, so keys
/// containing `"` are not supported there.
#[function(crate = ::rupa_core, requires = JsonPath, eval = eval_json_has_key)]
pub fn json_has_key(
    dialect: &dyn Dialect,
    doc: Expr<Json>,
    key: Expr<String>,
) -> Result<Expr<bool>, DslError> {
    let text = |s: &str| Expr::<String>::from_node(ExprNode::Param(Value::Text(s.into())));
    let any = |e: Expr<Json>| Expr::<String>::from_node(e.into_node());
    match dialect.id() {
        DialectId::Postgres => Ok(Expr::call("jsonb_exists", [any(doc), key])),
        DialectId::MySql => {
            let path = Expr::<String>::call("CONCAT", [text("$.\""), key, text("\"")]);
            Ok(Expr::call(
                "JSON_CONTAINS_PATH",
                [any(doc), text("one"), path],
            ))
        }
        DialectId::Sqlite => {
            let path = Expr::<String>::raw_op(
                "||",
                Expr::<String>::raw_op("||", text("$.\""), key),
                text("\""),
            );
            Ok(Expr::<Option<String>>::call("json_type", [any(doc), path]).is_not_null())
        }
        other => Err(DslError::unsupported("json_has_key", other)),
    }
}

fn eval_json_has_key(args: &[Value]) -> Result<Value, DslError> {
    Ok(match args {
        [Value::Null(_), _] | [_, Value::Null(_)] => Value::Null(SqlType::Bool),
        [Value::Json(doc), Value::Text(key)] => {
            Value::Bool(doc.as_object().is_some_and(|o| o.contains_key(key)))
        }
        other => {
            return Err(DslError::Other(format!(
                "`json_has_key` expects (JSON, text), got {other:?}"
            )));
        }
    })
}
