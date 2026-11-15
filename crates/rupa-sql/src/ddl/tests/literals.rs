use rupa_core::ir::ExprNode;

use super::expressions::{column, on_expression, param};
use super::*;

fn divided_by(value: Value) -> Index {
    on_expression(ExprNode::RawOp(
        "/",
        Box::new(column("n")),
        Box::new(param(value)),
    ))
}

fn rendered(value: Value, dialect: &dyn Dialect) -> String {
    render_schema(&SchemaOp::CreateIndex(divided_by(value)), dialect).expect("renders")
}

#[test]
fn float_literals_always_read_as_floats() {
    for (value, text) in [
        (Value::F64(2.0), "2.0"),
        (Value::F64(-0.0), "-0.0"),
        (Value::F64(0.5), "0.5"),
        (Value::F64(f64::MAX), "1.7976931348623157e308"),
        (Value::F64(1e-300), "1e-300"),
        (Value::F64(f64::MIN_POSITIVE), "2.2250738585072014e-308"),
        (Value::F32(3.0), "3.0"),
        (Value::F32(0.1), "0.10000000149011612"),
        (Value::F32(f32::MAX), "3.4028234663852886e38"),
    ] {
        let [postgres, mysql] = match value {
            Value::F32(_) => ["REAL", "FLOAT"],
            _ => ["DOUBLE PRECISION", "DOUBLE"],
        };

        for (dialect, expected) in [
            (
                &Postgres as &dyn Dialect,
                format!(" / CAST({text} AS {postgres})))"),
            ),
            (&MySql, format!(" / CAST({text} AS {mysql})))")),
            (&Sqlite, format!(" / {text}))")),
        ] {
            assert!(
                rendered(value.clone(), dialect).ends_with(&expected),
                "{value:?} on {dialect:?}"
            );
        }
    }
}

#[test]
fn integers_stay_integer_literals() {
    for value in [Value::I16(2), Value::I32(2), Value::I64(2)] {
        for dialect in [&MySql as &dyn Dialect, &Sqlite] {
            assert!(
                rendered(value.clone(), dialect).ends_with(" / 2))"),
                "{value:?} on {dialect:?}"
            );
        }
    }

    assert!(rendered(Value::I32(2), &Postgres).ends_with(r#""n" / 2))"#));
}

#[test]
fn postgres_keeps_small_and_big_integer_literals_their_bound_type() {
    assert!(rendered(Value::I16(2), &Postgres).ends_with(r#""n" / CAST(2 AS SMALLINT)))"#));
    assert!(
        rendered(Value::I64(1_000_000), &Postgres).ends_with(r#""n" / CAST(1000000 AS BIGINT)))"#)
    );
}

#[test]
fn integer_defaults_stay_bare() {
    let table = Table::at(NOTES).column(
        ColumnDef::new("count", ColumnType::BigInt).default(ColumnDefault::Value(Value::I64(5))),
    );

    for sql in ok(&SchemaOp::CreateTable(table)) {
        assert!(sql.contains("DEFAULT 5"), "{sql}");
    }
}

#[test]
fn float_defaults_are_written_as_float_literals() {
    let table = Table::at(NOTES).column(
        ColumnDef::new("ratio", ColumnType::Double).default(ColumnDefault::Value(Value::F64(1.0))),
    );

    for sql in ok(&SchemaOp::CreateTable(table)) {
        assert!(sql.contains("DEFAULT 1.0"), "{sql}");
    }
}
