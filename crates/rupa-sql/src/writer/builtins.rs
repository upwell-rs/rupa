//! Built-in functions a DDL expression cannot hold, per dialect.
//!
//! The lists hold each dialect's built-in aggregates and window functions,
//! and its well-known volatile functions. A user-defined aggregate is not
//! known here and is refused by the database when the migration runs.
//!
//! Only the target dialect's built-ins are refused, so a function another
//! dialect happens to define (SQLite's aggregate `total`) stays usable. A
//! schema-qualified name (`billing.total`) is a user function and is left to
//! the database, except PostgreSQL's `pg_catalog`, which holds its built-ins.

use rupa_core::DialectId;

/// Why a built-in cannot appear in a DDL expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Builtin {
    /// It aggregates rows.
    Aggregate,
    /// It is a window function.
    Window,
    /// Its result changes between calls with the same arguments.
    Volatile,
}

const PG: &[DialectId] = &[DialectId::Postgres];
const MY: &[DialectId] = &[DialectId::MySql];
const LITE: &[DialectId] = &[DialectId::Sqlite];
const PG_MY: &[DialectId] = &[DialectId::Postgres, DialectId::MySql];
const PG_LITE: &[DialectId] = &[DialectId::Postgres, DialectId::Sqlite];
const MY_LITE: &[DialectId] = &[DialectId::MySql, DialectId::Sqlite];
const ALL: &[DialectId] = &[DialectId::Postgres, DialectId::MySql, DialectId::Sqlite];

const AGGREGATES: &[(&str, &[DialectId])] = &[
    ("any_value", PG),
    ("array_agg", PG),
    ("avg", ALL),
    ("bit_and", PG_MY),
    ("bit_or", PG_MY),
    ("bit_xor", PG_MY),
    ("bool_and", PG),
    ("bool_or", PG),
    ("corr", PG),
    ("count", ALL),
    ("covar_pop", PG),
    ("covar_samp", PG),
    ("every", PG),
    ("group_concat", MY_LITE),
    ("grouping", PG_MY),
    ("json_agg", PG),
    ("json_agg_strict", PG),
    ("json_arrayagg", PG_MY),
    ("json_group_array", LITE),
    ("json_group_object", LITE),
    ("json_object_agg", PG),
    ("json_object_agg_strict", PG),
    ("json_object_agg_unique", PG),
    ("json_object_agg_unique_strict", PG),
    ("json_objectagg", PG_MY),
    ("jsonb_agg", PG),
    ("jsonb_agg_strict", PG),
    ("jsonb_group_array", LITE),
    ("jsonb_group_object", LITE),
    ("jsonb_object_agg", PG),
    ("jsonb_object_agg_strict", PG),
    ("jsonb_object_agg_unique", PG),
    ("jsonb_object_agg_unique_strict", PG),
    ("max", ALL),
    ("min", ALL),
    ("mode", PG),
    ("percentile_cont", PG),
    ("percentile_disc", PG),
    ("range_agg", PG),
    ("range_intersect_agg", PG),
    ("regr_avgx", PG),
    ("regr_avgy", PG),
    ("regr_count", PG),
    ("regr_intercept", PG),
    ("regr_r2", PG),
    ("regr_slope", PG),
    ("regr_sxx", PG),
    ("regr_sxy", PG),
    ("regr_syy", PG),
    ("std", MY),
    ("stddev", PG_MY),
    ("stddev_pop", PG_MY),
    ("stddev_samp", PG_MY),
    ("string_agg", PG_LITE),
    ("sum", ALL),
    ("total", LITE),
    ("var_pop", PG_MY),
    ("var_samp", PG_MY),
    ("variance", PG_MY),
    ("xmlagg", PG),
];

const WINDOWS: &[(&str, &[DialectId])] = &[
    ("cume_dist", ALL),
    ("dense_rank", ALL),
    ("first_value", ALL),
    ("lag", ALL),
    ("last_value", ALL),
    ("lead", ALL),
    ("nth_value", ALL),
    ("ntile", ALL),
    ("percent_rank", ALL),
    ("rank", ALL),
    ("row_number", ALL),
];

const VOLATILE: &[(&str, &[DialectId])] = &[
    ("changes", LITE),
    ("clock_timestamp", PG),
    ("connection_id", MY),
    ("curdate", MY),
    ("current_date", ALL),
    ("current_time", ALL),
    ("current_timestamp", ALL),
    ("current_user", PG_MY),
    ("curtime", MY),
    ("currval", PG),
    ("database", MY),
    ("found_rows", MY),
    ("gen_random_uuid", PG),
    ("last_insert_id", MY),
    ("last_insert_rowid", LITE),
    ("lastval", PG),
    ("localtime", PG_MY),
    ("localtimestamp", PG_MY),
    ("nextval", PG),
    ("now", PG_MY),
    ("rand", MY),
    ("random", PG_LITE),
    ("randomblob", LITE),
    ("row_count", MY),
    ("session_user", PG_MY),
    ("setval", PG),
    ("statement_timestamp", PG),
    ("sysdate", MY),
    ("timeofday", PG),
    ("total_changes", LITE),
    ("transaction_timestamp", PG),
    ("user", MY),
    ("utc_date", MY),
    ("utc_time", MY),
    ("utc_timestamp", MY),
    ("uuid", MY),
    ("uuid_generate_v4", PG),
    ("uuid_short", MY),
    ("version", PG_MY),
];

/// The kind of built-in `name` is on `dialect`, called with `arguments`
/// arguments, if DDL cannot hold it. SQLite's `max` and `min` with several
/// arguments are scalar functions; with one they aggregate. PostgreSQL and
/// MySQL spell the scalar form `greatest`/`least`, so there the name is
/// always an aggregate.
pub(crate) fn classify(name: &str, arguments: usize, dialect: DialectId) -> Option<Builtin> {
    let bare = builtin_name(name, dialect)?;
    let listed = |list: &[(&str, &[DialectId])]| {
        list.iter()
            .any(|(builtin, on)| builtin.eq_ignore_ascii_case(bare) && on.contains(&dialect))
    };
    let scalar = dialect == DialectId::Sqlite
        && arguments > 1
        && (bare.eq_ignore_ascii_case("max") || bare.eq_ignore_ascii_case("min"));

    if listed(AGGREGATES) && !scalar {
        return Some(Builtin::Aggregate);
    }

    if listed(WINDOWS) {
        return Some(Builtin::Window);
    }

    listed(VOLATILE).then_some(Builtin::Volatile)
}

/// The name to look up among `dialect`'s built-ins, or `None` for a user
/// function in another schema.
fn builtin_name(name: &str, dialect: DialectId) -> Option<&str> {
    match name.rsplit_once('.') {
        None => Some(name),
        Some((schema, bare))
            if dialect == DialectId::Postgres && schema.eq_ignore_ascii_case("pg_catalog") =>
        {
            Some(bare)
        }
        Some(_) => None,
    }
}
