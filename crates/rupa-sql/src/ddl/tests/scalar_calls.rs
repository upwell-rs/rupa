use super::expressions::{DIALECTS, call, column, on_expression, refused};
use super::*;
use crate::ExprRefusal;

fn two_argument(name: &'static str) -> Index {
    on_expression(call(name, vec![column("a"), column("b")]))
}

#[test]
fn sqlite_max_and_min_with_several_arguments_are_scalar() {
    for name in ["max", "MIN"] {
        let index = two_argument(name);

        assert_eq!(
            render_schema(&SchemaOp::CreateIndex(index), &Sqlite).as_deref(),
            Ok(
                format!(r#"CREATE INDEX "app"."notes_expr" ON "notes" (({name}("a", "b")))"#)
                    .as_str()
            )
        );
    }
}

#[test]
fn sqlite_max_and_min_with_one_argument_still_aggregate() {
    for name in ["max", "min"] {
        let index = on_expression(call(name, vec![column("a")]));

        assert_eq!(
            render_schema(&SchemaOp::CreateIndex(index), &Sqlite),
            refused(
                "notes_expr",
                DialectId::Sqlite,
                ExprRefusal::Aggregate { function: name }
            )
        );
    }
}

#[test]
fn other_dialects_refuse_max_and_min_by_name() {
    let index = two_argument("max");

    for (dialect, rendered) in DIALECTS
        .into_iter()
        .zip(all(&SchemaOp::CreateIndex(index)))
        .filter(|(dialect, _)| *dialect != DialectId::Sqlite)
    {
        assert_eq!(
            rendered,
            refused(
                "notes_expr",
                dialect,
                ExprRefusal::Aggregate { function: "max" }
            )
        );
    }
}

#[test]
fn other_aggregates_stay_refused_on_sqlite_whatever_their_arguments() {
    let index = on_expression(call("sum", vec![column("a"), column("b")]));

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Sqlite),
        refused(
            "notes_expr",
            DialectId::Sqlite,
            ExprRefusal::Aggregate { function: "sum" }
        )
    );
}

#[test]
fn only_the_target_dialects_builtins_are_refused() {
    let index = on_expression(call("total", vec![column("a")]));

    for (dialect, rendered) in DIALECTS.into_iter().zip(all(&SchemaOp::CreateIndex(index))) {
        match dialect {
            DialectId::Sqlite => assert_eq!(
                rendered,
                refused(
                    "notes_expr",
                    dialect,
                    ExprRefusal::Aggregate { function: "total" }
                )
            ),
            _ => assert!(rendered.is_ok(), "{dialect:?}: {rendered:?}"),
        }
    }
}

#[test]
fn schema_qualified_functions_are_user_functions() {
    let index = on_expression(call("billing.total", vec![column("a"), column("b")]));

    for (dialect, rendered) in DIALECTS.into_iter().zip(all(&SchemaOp::CreateIndex(index))) {
        assert!(rendered.is_ok(), "{dialect:?}: {rendered:?}");
    }
}

#[test]
fn postgres_still_refuses_pg_catalog_builtins() {
    let index = on_expression(call("pg_catalog.count", vec![column("a")]));

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Postgres),
        refused(
            "notes_expr",
            DialectId::Postgres,
            ExprRefusal::Aggregate {
                function: "pg_catalog.count"
            }
        )
    );
}

#[test]
fn volatile_builtins_are_refused_where_the_dialect_defines_them() {
    let [postgres, mysql, sqlite] = [
        call("pg_catalog.random", vec![]),
        call("rand", vec![]),
        call("random", vec![]),
    ]
    .map(on_expression);

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(postgres), &Postgres),
        refused(
            "notes_expr",
            DialectId::Postgres,
            ExprRefusal::Volatile {
                function: "pg_catalog.random"
            }
        )
    );
    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(mysql), &MySql),
        refused(
            "notes_expr",
            DialectId::MySql,
            ExprRefusal::Volatile { function: "rand" }
        )
    );
    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(sqlite), &Sqlite),
        refused(
            "notes_expr",
            DialectId::Sqlite,
            ExprRefusal::Volatile { function: "random" }
        )
    );
}

#[test]
fn statistical_and_json_aggregates_are_refused() {
    for (name, dialect) in [
        ("stddev", &Postgres as &dyn Dialect),
        ("variance", &MySql),
        ("std", &MySql),
        ("jsonb_object_agg", &Postgres),
        ("regr_slope", &Postgres),
        ("jsonb_group_array", &Sqlite),
    ] {
        let index = on_expression(call(name, vec![column("a")]));

        assert_eq!(
            render_schema(&SchemaOp::CreateIndex(index), dialect),
            refused(
                "notes_expr",
                dialect.id(),
                ExprRefusal::Aggregate { function: name }
            ),
            "{name} on {dialect:?}"
        );
    }
}
