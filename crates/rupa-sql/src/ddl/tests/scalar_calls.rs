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
