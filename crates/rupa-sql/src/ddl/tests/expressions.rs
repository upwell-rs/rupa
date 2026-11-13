use rupa_core::SqlType;
use rupa_core::column::{ColumnMeta, StorageKind};
use rupa_core::ir::{BinOp, ColumnRef, ExprNode, SourceId, UnOp};
use rupa_core::schema::{DdlExpr, IndexExpr, IndexPart};

use super::*;
use crate::ExprRefusal;

pub(super) const DIALECTS: [DialectId; 3] =
    [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite];

const SCOPE: &[ColumnMeta] = &[
    ColumnMeta::new("title", "title", StorageKind::Scalar, SqlType::Text, false),
    ColumnMeta::new("n", "n", StorageKind::Scalar, SqlType::I64, false),
];

pub(super) fn column(name: &'static str) -> ExprNode {
    ExprNode::Column(ColumnRef::new(name))
}

pub(super) fn call(name: &'static str, args: Vec<ExprNode>) -> ExprNode {
    ExprNode::Call(name, args)
}

pub(super) fn param(value: Value) -> ExprNode {
    ExprNode::Param(value)
}

pub(super) fn binary(op: BinOp, lhs: ExprNode, rhs: ExprNode) -> ExprNode {
    ExprNode::Binary(op, Box::new(lhs), Box::new(rhs))
}

fn expression(node: ExprNode, ty: ColumnType) -> IndexPart {
    IndexExpr {
        expr: DdlExpr::new(node),
        ty,
    }
    .into()
}

pub(super) fn on_expression(node: ExprNode) -> Index {
    Index {
        parts: vec![expression(node, ColumnType::VarChar(200))],
        ..index("notes_expr", BTree::METHOD, [])
    }
}

pub(super) fn partial(node: ExprNode) -> Index {
    index(
        "notes_partial",
        BTree::METHOD,
        [("title", ColumnType::Text)],
    )
    .filter(DdlExpr::new(node))
}

pub(super) fn refused(
    object: &'static str,
    dialect: DialectId,
    reason: ExprRefusal,
) -> Result<String, RenderError> {
    Err(RenderError::Schema(Box::new(SchemaError::Expression {
        object,
        dialect,
        reason,
    })))
}

#[test]
fn expression_parts_render_in_extra_parentheses() {
    let index = on_expression(call("lower", vec![column("title")])).unique();

    assert_eq!(
        ok(&SchemaOp::CreateIndex(index)),
        [
            r#"CREATE UNIQUE INDEX "notes_expr" ON "app"."notes" ((lower("title")))"#,
            "CREATE UNIQUE INDEX `notes_expr` ON `app`.`notes` ((lower(`title`)))",
            r#"CREATE UNIQUE INDEX "app"."notes_expr" ON "notes" ((lower("title")))"#,
        ]
    );
}

#[test]
fn columns_and_expressions_mix_in_order() {
    let mut index = on_expression(call("lower", vec![column("title")]));

    index
        .parts
        .insert(0, IndexColumn::new("author_id", ColumnType::BigInt).into());

    assert_eq!(
        ok(&SchemaOp::CreateIndex(index))[0],
        r#"CREATE INDEX "notes_expr" ON "app"."notes" ("author_id", (lower("title")))"#
    );
}

#[test]
fn partial_indexes_render_where_on_postgres_and_sqlite() {
    let index = partial(ExprNode::Unary(UnOp::IsNull, Box::new(column("body"))));
    let [postgres, mysql, sqlite] = all(&SchemaOp::CreateIndex(index));

    assert_eq!(
        postgres.as_deref(),
        Ok(r#"CREATE INDEX "notes_partial" ON "app"."notes" ("title") WHERE "body" IS NULL"#)
    );
    assert_eq!(
        sqlite.as_deref(),
        Ok(r#"CREATE INDEX "app"."notes_partial" ON "notes" ("title") WHERE "body" IS NULL"#)
    );
    assert_eq!(
        mysql,
        Err(refusal(
            "notes_partial",
            DialectId::MySql,
            IndexRefusal::Partial
        ))
    );
}

#[test]
fn literals_are_inlined_as_checked_literals() {
    let node = binary(
        BinOp::And,
        binary(
            BinOp::Eq,
            column("title"),
            param(Value::Text("it's".into())),
        ),
        binary(BinOp::Gt, column("n"), param(Value::I64(-3))),
    );
    let [postgres, _, sqlite] = all(&SchemaOp::CreateIndex(partial(node)));

    assert_eq!(
        postgres.as_deref(),
        Ok(
            r#"CREATE INDEX "notes_partial" ON "app"."notes" ("title") WHERE "title" = 'it''s' AND "n" > -3"#
        )
    );
    assert_eq!(
        sqlite.as_deref(),
        Ok(
            r#"CREATE INDEX "app"."notes_partial" ON "notes" ("title") WHERE "title" = 'it''s' AND "n" > -3"#
        )
    );
}

#[test]
fn booleans_and_nulls_follow_the_dialect() {
    let node = binary(BinOp::Eq, column("pinned"), param(Value::Bool(true)));
    let [postgres, _, sqlite] = all(&SchemaOp::CreateIndex(partial(node)));
    let null = partial(binary(
        BinOp::Eq,
        column("body"),
        param(Value::Null(SqlType::Text)),
    ));

    assert!(
        postgres
            .expect("renders")
            .ends_with(r#"WHERE "pinned" = TRUE"#)
    );
    assert!(sqlite.expect("renders").ends_with(r#"WHERE "pinned" = 1"#));
    assert!(
        render_schema(&SchemaOp::CreateIndex(null), &Postgres)
            .expect("renders")
            .ends_with(r#"WHERE "body" = NULL"#),
        "a typed null is written as NULL"
    );
}

#[test]
fn backslashes_follow_each_dialects_literal_rules() {
    let node = binary(
        BinOp::Eq,
        column("title"),
        param(Value::Text(r"a\b".into())),
    );
    let [postgres, mysql, sqlite] = all(&SchemaOp::CreateIndex(on_expression(node)));

    assert_eq!(
        postgres.as_deref(),
        Ok(r#"CREATE INDEX "notes_expr" ON "app"."notes" (("title" = E'a\\b'))"#)
    );
    assert_eq!(
        sqlite.as_deref(),
        Ok(r#"CREATE INDEX "app"."notes_expr" ON "notes" (("title" = 'a\b'))"#)
    );
    assert_eq!(
        mysql,
        refused(
            "notes_expr",
            DialectId::MySql,
            ExprRefusal::Literal { ty: SqlType::Text }
        )
    );
}

#[test]
fn values_without_a_safe_literal_are_refused() {
    for value in [
        Value::Bytes(vec![1]),
        Value::F64(f64::NAN),
        Value::Text("a\0b".into()),
    ] {
        let ty = value.sql_type();
        let index = partial(binary(BinOp::Eq, column("title"), param(value)));

        for (dialect, rendered) in DIALECTS.into_iter().zip(all(&SchemaOp::CreateIndex(index))) {
            let expected = if dialect == DialectId::MySql {
                refusal("notes_partial", dialect, IndexRefusal::Partial)
            } else {
                refused("notes_partial", dialect, ExprRefusal::Literal { ty }).expect_err("refused")
            };

            assert_eq!(rendered, Err(expected));
        }
    }
}

#[test]
fn calls_ddl_cannot_hold_are_refused_with_their_kind() {
    for (node, reason) in [
        (
            call("count", vec![column("n")]),
            ExprRefusal::Aggregate { function: "count" },
        ),
        (
            call("SUM", vec![column("n")]),
            ExprRefusal::Aggregate { function: "SUM" },
        ),
        (
            call("row_number", vec![]),
            ExprRefusal::Window {
                function: "row_number",
            },
        ),
        (
            call("now", vec![]),
            ExprRefusal::Volatile { function: "now" },
        ),
        (
            call("pg_catalog.random", vec![]),
            ExprRefusal::Volatile {
                function: "pg_catalog.random",
            },
        ),
        (
            call("lower", vec![call("RANDOM", vec![])]),
            ExprRefusal::Volatile { function: "RANDOM" },
        ),
    ] {
        let rendered = all(&SchemaOp::CreateIndex(on_expression(node.clone())));

        for (dialect, rendered) in DIALECTS.into_iter().zip(rendered) {
            assert_eq!(rendered, refused("notes_expr", dialect, reason), "{node:?}");
        }
    }
}

#[test]
fn predicates_are_checked_like_index_expressions() {
    let [postgres, _, sqlite] = all(&SchemaOp::CreateIndex(partial(binary(
        BinOp::Gt,
        call("count", vec![column("n")]),
        param(Value::I64(1)),
    ))));

    assert_eq!(
        postgres,
        refused(
            "notes_partial",
            DialectId::Postgres,
            ExprRefusal::Aggregate { function: "count" }
        )
    );
    assert_eq!(
        sqlite,
        refused(
            "notes_partial",
            DialectId::Sqlite,
            ExprRefusal::Aggregate { function: "count" }
        )
    );
}

#[test]
fn columns_outside_the_table_are_refused() {
    let scoped = |node: ExprNode| Index {
        parts: vec![
            IndexExpr {
                expr: DdlExpr {
                    node,
                    scope: Some(SCOPE),
                },
                ty: ColumnType::VarChar(200),
            }
            .into(),
        ],
        ..index("notes_expr", BTree::METHOD, [])
    };

    assert_eq!(
        ok(&SchemaOp::CreateIndex(scoped(column("title"))))[0],
        r#"CREATE INDEX "notes_expr" ON "app"."notes" (("title"))"#
    );

    for node in [
        column("price"),
        ExprNode::Column(ColumnRef::of(SourceId(1), "title")),
    ] {
        let column = match &node {
            ExprNode::Column(c) => c.name,
            _ => unreachable!("built above"),
        };

        assert_eq!(
            render_schema(&SchemaOp::CreateIndex(scoped(node)), &Postgres),
            refused(
                "notes_expr",
                DialectId::Postgres,
                ExprRefusal::Column {
                    column,
                    table: None,
                    expected: NOTES
                }
            )
        );
    }
}

#[test]
fn mysql_refuses_expressions_of_unbounded_type() {
    for ty in [ColumnType::Text, ColumnType::Bytes, ColumnType::Json] {
        let index = Index {
            parts: vec![expression(call("lower", vec![column("title")]), ty)],
            ..index("notes_expr", BTree::METHOD, [])
        };
        let [postgres, mysql, sqlite] = all(&SchemaOp::CreateIndex(index));

        assert!(postgres.is_ok() && sqlite.is_ok());
        assert_eq!(
            mysql,
            Err(refusal(
                "notes_expr",
                DialectId::MySql,
                IndexRefusal::UnboundedExpr { ty }
            ))
        );
    }
}

#[test]
fn mysql_accepts_expressions_of_bounded_type() {
    let index = Index {
        parts: vec![expression(
            call("abs", vec![column("n")]),
            ColumnType::BigInt,
        )],
        ..index("notes_expr", BTree::METHOD, [])
    };

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &MySql).as_deref(),
        Ok("CREATE INDEX `notes_expr` ON `app`.`notes` ((abs(`n`)))")
    );
}

#[test]
fn expressions_follow_the_kinds_limits() {
    let hash = Index {
        parts: vec![expression(
            call("lower", vec![column("title")]),
            ColumnType::VarChar(10),
        )],
        ..index("notes_hash", Hash::METHOD, [])
    };

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(hash), &Postgres).as_deref(),
        Ok(r#"CREATE INDEX "notes_hash" ON "app"."notes" USING hash ((lower("title")))"#)
    );
}
