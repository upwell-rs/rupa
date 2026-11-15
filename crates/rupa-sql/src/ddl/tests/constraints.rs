use rupa_core::ir::{BinOp, ExprNode};
use rupa_core::schema::{Check, Constraint, ConstraintKind, DdlExpr, Unique};

use super::expressions::{DIALECTS, SCOPE, binary, call, column, param, refused};
use super::*;
use crate::{ConstraintChange, ExprRefusal};

fn positive() -> Check {
    Check::new(
        "notes_author_positive",
        DdlExpr::new(binary(BinOp::Gt, column("author_id"), param(Value::I64(0)))),
    )
}

fn titled() -> Check {
    Check::new(
        "notes_title_nonempty",
        DdlExpr::new(binary(
            BinOp::Ne,
            column("title"),
            param(Value::Text(String::new())),
        )),
    )
}

fn title_author() -> Unique {
    Unique::new(
        "notes_title_author",
        [
            IndexColumn::new("title", ColumnType::VarChar(200)),
            IndexColumn::new("author_id", ColumnType::BigInt),
        ],
    )
}

fn body_unique() -> Unique {
    Unique::new("notes_body", [IndexColumn::new("body", ColumnType::Text)])
}

fn constrained() -> Table {
    notes().check(titled()).unique(title_author())
}

fn alteration(error: Result<String, RenderError>) -> (&'static str, DialectId, ConstraintChange) {
    match error {
        Err(RenderError::Schema(error)) => match *error {
            SchemaError::ConstraintAlteration {
                constraint,
                dialect,
                change,
            } => (constraint, dialect, change),
            other => panic!("expected a constraint alteration, got {other:?}"),
        },
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn create_table_writes_named_constraints_after_its_keys() {
    let [postgres, mysql, sqlite] = ok(&SchemaOp::CreateTable(constrained()));

    assert!(
        postgres.ends_with(
            r#"ON DELETE SET NULL, CONSTRAINT "notes_title_nonempty" CHECK ("title" <> ''), CONSTRAINT "notes_title_author" UNIQUE ("title", "author_id"))"#
        ),
        "{postgres}"
    );
    assert!(
        mysql.ends_with(
            "ON DELETE SET NULL, CONSTRAINT `notes_title_nonempty` CHECK (`title` <> ''), CONSTRAINT `notes_title_author` UNIQUE (`title`, `author_id`))"
        ),
        "{mysql}"
    );
    assert!(
        sqlite.ends_with(
            r#"ON DELETE SET NULL, CONSTRAINT "notes_title_nonempty" CHECK ("title" <> ''), CONSTRAINT "notes_title_author" UNIQUE ("title", "author_id"))"#
        ),
        "{sqlite}"
    );
}

#[test]
fn adding_a_constraint_renders_on_postgres_and_mysql() {
    let check = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: positive().into(),
    };
    let unique = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: title_author().into(),
    };

    assert_eq!(
        [
            render_schema(&check, &Postgres).as_deref(),
            render_schema(&check, &MySql).as_deref(),
            render_schema(&unique, &Postgres).as_deref(),
            render_schema(&unique, &MySql).as_deref(),
        ],
        [
            Ok(
                r#"ALTER TABLE "app"."notes" ADD CONSTRAINT "notes_author_positive" CHECK ("author_id" > CAST(0 AS BIGINT))"#
            ),
            Ok(
                "ALTER TABLE `app`.`notes` ADD CONSTRAINT `notes_author_positive` CHECK (`author_id` > 0)"
            ),
            Ok(
                r#"ALTER TABLE "app"."notes" ADD CONSTRAINT "notes_title_author" UNIQUE ("title", "author_id")"#
            ),
            Ok(
                "ALTER TABLE `app`.`notes` ADD CONSTRAINT `notes_title_author` UNIQUE (`title`, `author_id`)"
            ),
        ]
    );
}

#[test]
fn dropping_a_constraint_follows_each_dialects_grammar() {
    let drop = |kind| SchemaOp::DropConstraint {
        table: NOTES,
        name: "c",
        kind,
    };

    assert_eq!(
        [ConstraintKind::Check, ConstraintKind::Unique].map(|kind| (
            render_schema(&drop(kind), &Postgres).expect("postgres drops both"),
            render_schema(&drop(kind), &MySql).expect("mysql drops both"),
        )),
        [
            (
                r#"ALTER TABLE "app"."notes" DROP CONSTRAINT "c""#.to_owned(),
                "ALTER TABLE `app`.`notes` DROP CHECK `c`".to_owned(),
            ),
            (
                r#"ALTER TABLE "app"."notes" DROP CONSTRAINT "c""#.to_owned(),
                "ALTER TABLE `app`.`notes` DROP INDEX `c`".to_owned(),
            ),
        ]
    );
}

#[test]
fn sqlite_refuses_to_add_or_drop_a_constraint() {
    let add = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: Constraint::Unique(title_author()),
    };
    let drop = SchemaOp::DropConstraint {
        table: NOTES,
        name: "c",
        kind: ConstraintKind::Check,
    };

    assert_eq!(
        alteration(render_schema(&add, &Sqlite)),
        (
            "notes_title_author",
            DialectId::Sqlite,
            ConstraintChange::Add
        )
    );
    assert_eq!(
        alteration(render_schema(&drop, &Sqlite)),
        ("c", DialectId::Sqlite, ConstraintChange::Drop)
    );
}

#[test]
fn mysql_refuses_unbounded_columns_in_a_unique_constraint() {
    let add = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: body_unique().into(),
    };
    let create = SchemaOp::CreateTable(notes().unique(body_unique()));
    let expected = RenderError::Schema(Box::new(SchemaError::Unindexable {
        column: "body",
        ty: ColumnType::Text,
        dialect: DialectId::MySql,
    }));

    assert_eq!(render_schema(&add, &MySql), Err(expected.clone()));
    assert_eq!(render_schema(&create, &MySql), Err(expected));
    assert!(render_schema(&add, &Postgres).is_ok());
    assert!(render_schema(&create, &Sqlite).is_ok());
}

#[test]
fn a_created_unique_is_checked_against_the_tables_own_column_types() {
    let retyped = notes()
        .column(ColumnDef::new("body", ColumnType::VarChar(100)).nullable())
        .unique(body_unique());

    assert!(render_schema(&SchemaOp::CreateTable(retyped), &MySql).is_ok());
}

#[test]
fn a_unique_over_a_missing_column_is_refused_in_create_table() {
    let table = notes().unique(Unique::new(
        "notes_ghost",
        [IndexColumn::new("ghost", ColumnType::Int)],
    ));

    for result in all(&SchemaOp::CreateTable(table)) {
        assert_eq!(
            result,
            Err(RenderError::Schema(Box::new(SchemaError::UnknownColumn {
                table: "notes",
                column: "ghost",
            })))
        );
    }
}

#[test]
fn a_unique_without_columns_is_refused() {
    let op = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: Unique::new("notes_none", []).into(),
    };

    assert_eq!(
        render_schema(&op, &Postgres),
        Err(RenderError::Schema(Box::new(SchemaError::EmptyConstraint(
            "notes_none"
        ))))
    );
}

#[test]
fn a_check_refuses_what_ddl_cannot_hold_naming_the_constraint() {
    let aggregate = Check::new(
        "notes_counted",
        DdlExpr::new(binary(
            BinOp::Gt,
            call("count", vec![column("title")]),
            param(Value::I64(0)),
        )),
    );
    let table = notes().check(aggregate);

    for (dialect, result) in DIALECTS.into_iter().zip(all(&SchemaOp::CreateTable(table))) {
        assert_eq!(
            result,
            refused(
                "notes_counted",
                dialect,
                ExprRefusal::Aggregate { function: "count" }
            )
        );
    }
}

#[test]
fn a_typed_check_refuses_a_column_outside_its_scope() {
    let foreign = Check::new(
        "notes_foreign",
        DdlExpr {
            node: ExprNode::Column(rupa_core::ir::ColumnRef::new("price")),
            scope: Some(SCOPE),
        },
    );
    let op = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: foreign.into(),
    };

    assert_eq!(
        render_schema(&op, &Postgres),
        refused(
            "notes_foreign",
            DialectId::Postgres,
            ExprRefusal::Column {
                column: "price",
                table: None,
                expected: NOTES,
            }
        )
    );
}

#[test]
fn a_table_without_constraints_renders_as_before() {
    let [postgres, ..] = ok(&SchemaOp::CreateTable(notes()));

    assert!(!postgres.contains("CONSTRAINT"), "{postgres}");
}

fn check_on(name: &'static str, column_name: &'static str) -> Check {
    Check::new(
        name,
        DdlExpr::new(binary(BinOp::Gt, column(column_name), param(Value::I64(0)))),
    )
}

#[test]
fn mysql_refuses_a_check_over_the_auto_increment_column() {
    let table = notes().check(check_on("notes_id_positive", "id"));
    let results = all(&SchemaOp::CreateTable(table));

    assert_eq!(
        results[1],
        refused(
            "notes_id_positive",
            DialectId::MySql,
            ExprRefusal::AutoIncrementColumn { column: "id" }
        )
    );
    assert!(results[0].is_ok() && results[2].is_ok());
}

#[test]
fn mysql_refuses_a_check_over_a_cascading_foreign_key_column() {
    for action in [ReferentialAction::Cascade, ReferentialAction::SetNull] {
        let table = notes()
            .foreign_key(
                ForeignKey::new(
                    ["author_id"],
                    Reference {
                        table: TableRef::with_schema("app", "authors"),
                        columns: &["id"],
                    },
                )
                .on_delete(action),
            )
            .check(check_on("notes_author_positive", "author_id"));

        assert_eq!(
            render_schema(&SchemaOp::CreateTable(table), &MySql),
            refused(
                "notes_author_positive",
                DialectId::MySql,
                ExprRefusal::ReferentialActionColumn {
                    column: "author_id"
                }
            )
        );
    }
}

#[test]
fn mysql_allows_a_check_over_a_restricting_foreign_key_column() {
    for action in [ReferentialAction::NoAction, ReferentialAction::Restrict] {
        let table = notes()
            .foreign_key(
                ForeignKey::new(
                    ["author_id"],
                    Reference {
                        table: TableRef::with_schema("app", "authors"),
                        columns: &["id"],
                    },
                )
                .on_delete(action),
            )
            .check(check_on("notes_author_positive", "author_id"));

        assert!(render_schema(&SchemaOp::CreateTable(table), &MySql).is_ok());
    }
}

#[test]
fn a_restricted_column_is_not_refused_when_added_to_an_existing_table() {
    let op = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: check_on("notes_id_positive", "id").into(),
    };

    assert!(render_schema(&op, &MySql).is_ok());
}

fn wrong_table() -> SchemaError {
    SchemaError::ConstraintTable {
        constraint: "notes_other",
        recorded: TableRef::new("other"),
        expected: NOTES,
    }
}

fn other_unique() -> Unique {
    Unique {
        table: Some(TableRef::new("other")),
        ..Unique::new("notes_other", [IndexColumn::new("title", ColumnType::Text)])
    }
}

#[test]
fn a_unique_recorded_for_another_table_is_refused_everywhere() {
    let create = SchemaOp::CreateTable(notes().unique(other_unique()));
    let add = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: other_unique().into(),
    };

    for result in all(&create) {
        assert_eq!(result, Err(RenderError::Schema(Box::new(wrong_table()))));
    }

    for dialect in [&Postgres as &dyn Dialect, &MySql] {
        assert_eq!(
            render_schema(&add, dialect),
            Err(RenderError::Schema(Box::new(wrong_table())))
        );
    }
}

#[test]
fn a_unique_recorded_for_its_own_table_or_none_is_accepted() {
    let own = Unique {
        table: Some(NOTES),
        ..other_unique()
    };

    assert!(render_schema(&SchemaOp::CreateTable(notes().unique(own)), &Postgres).is_ok());
    assert!(
        render_schema(
            &SchemaOp::CreateTable(notes().unique(Unique {
                table: None,
                ..other_unique()
            })),
            &Postgres
        )
        .is_ok()
    );
}
