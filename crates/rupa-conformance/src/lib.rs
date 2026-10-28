//! Behavioural test suite shared by every driver. Each driver's tests supply a
//! [`Harness`] and call [`run`]. Because the same assertions run against the
//! memory backend and real databases, passing everywhere means the backends agree.
//!
//! Cases are written once, against [`AsyncExecutor`]. Sync drivers run them
//! through [`Blocking`], which still calls the driver's own `Executor::execute`.

pub mod database;
pub mod fixture;
mod joins;
#[cfg(feature = "migrations")]
pub mod migrations;

use std::fmt::Debug;
use std::future::Future;

use rupa::Dialect;
use rupa::core::ir::ExprNode;
use rupa::core::{Capability, DialectId, DslError, ResultError, Value};
use rupa::prelude::*;
use rupa::{AsyncExecutor, ExecError};
use rupa::{AsyncTransaction, AsyncTransactional, IsolationLevel, TxOptions};

pub use database::{DatabaseHarness, run_database, run_pool_cases, run_script};
pub use fixture::{
    Item, MYSQL_DDL, NewNote, Note, NotePatch, POSTGRES_DDL, Review, SQLITE_ATTACH, SQLITE_DDL,
    SetPinned, items, reviews,
};

/// Gives the suite a fresh, empty schema per case.
pub trait Harness {
    type Exec: AsyncExecutor;

    /// Drops and recreates the conformance tables, empty.
    fn reset(&mut self) -> impl Future<Output = ()>;

    fn exec(&mut self) -> &mut Self::Exec;
}

pub use rupa::Blocking;

/// Runs every case, each on a freshly reset schema. The case name is printed
/// before it runs, so a failure's captured output says which case it was.
pub async fn run<H: Harness>(h: &mut H) {
    macro_rules! cases {
        ($module:ident: $($case:ident),* $(,)?) => {$(
            h.reset().await;
            eprintln!("conformance {}", stringify!($case));
            $module::$case(h.exec()).await;
        )*};
    }
    cases!(
        cases:
        case_insert_and_select,
        case_single_row_shapes,
        case_comparisons,
        case_three_valued_logic,
        case_in_lists,
        case_like_patterns,
        case_order_with_nulls,
        case_limit_offset,
        case_exists,
        case_update,
        case_delete,
        case_json_paths,
        case_nullable_json,
        case_timestamps,
        case_dsl_lowering_and_eval_agree,
        case_duplicate_key_is_an_error,
        case_failed_multi_row_insert_is_atomic,
        case_generated_ids_and_returning,
        case_get_by_id,
        case_update_entity_and_patches,
        case_delete_one_and_by_id,
        case_dsl_std_text,
        case_dsl_std_json,
    );
    cases!(
        joins:
        case_join_on_macro,
        case_join_declared_relations,
        case_join_into_merged_struct,
        case_join_query_attribute,
        case_join_self_join_aliases,
        case_join_inner,
        case_join_left,
        case_join_filter_and_order_on_joined_columns,
        case_join_self_left_join_decodes_none,
        case_join_three_sources,
        case_join_single_row_shapes_and_exists,
    );
}

/// Runs the transaction cases. Needs a transactional executor.
pub async fn run_transactions<H: Harness>(h: &mut H)
where
    H::Exec: AsyncTransactional,
{
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            h.reset().await;
            eprintln!("conformance {}", stringify!($case));
            tx_cases::$case(h.exec()).await;
        )*};
    }
    cases!(
        tx_commit_persists,
        tx_rollback_discards,
        tx_drop_rolls_back,
        tx_is_an_executor,
        tx_nested_rollback_keeps_outer,
        tx_nested_drop_keeps_outer,
        tx_nested_commit_outer_rollback,
        tx_closure_commits_or_rolls_back,
        tx_read_only_rejects_writes,
        tx_options_on_nested_are_rejected,
        tx_isolation_levels,
        tx_sequential_after_drop,
    );
}

/// `upper(a) = upper(b)`: lowered for every SQL dialect (standard SQL),
/// evaluated by the memory backend through `eval`.
#[rupa::dsl::function(eval = eval_ci_eq)]
pub fn ci_eq(
    dialect: &dyn Dialect,
    a: Expr<String>,
    b: Expr<String>,
) -> Result<Expr<bool>, DslError> {
    match dialect.id() {
        DialectId::Postgres | DialectId::MySql | DialectId::Sqlite => {
            let (a, b) = (
                Expr::<String>::call("upper", [a]),
                Expr::<String>::call("upper", [b]),
            );
            Ok(Expr::raw_op("=", a, b))
        }
        other => Err(DslError::unsupported("ci_eq", other)),
    }
}

fn eval_ci_eq(args: &[Value]) -> Result<Value, DslError> {
    match args {
        [Value::Text(a), Value::Text(b)] => Ok(Value::Bool(a.to_uppercase() == b.to_uppercase())),
        [Value::Null(_), _] | [_, Value::Null(_)] => Ok(Value::Null(rupa::SqlType::Bool)),
        other => Err(DslError::Other(format!(
            "ci_eq: unexpected arguments {other:?}"
        ))),
    }
}

fn result_error<T: Debug, E: ExecError>(r: Result<T, E>) -> ResultError {
    match r {
        Ok(v) => panic!("expected a result error, got Ok({v:?})"),
        Err(e) => e
            .result_error()
            .cloned()
            .unwrap_or_else(|| panic!("expected a result error, got {e}")),
    }
}

mod cases;

mod tx_cases;
