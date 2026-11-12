use rupa_core::SqlType;
use rupa_core::schema::{ColumnSpec, TypeSupport};

use super::*;

pub(super) const CITEXT: ColumnSpec = ColumnSpec::new("citext", SqlType::Text)
    .postgres(TypeSupport::named("CITEXT"))
    .mysql(TypeSupport::named("VARCHAR(255)"))
    .sqlite(TypeSupport::Unsupported("SQLite has no citext type"))
    .bounded();

const VECTOR: ColumnSpec = ColumnSpec::new("vector", SqlType::Text)
    .postgres(TypeSupport::with_args("vector", &[3, 4]))
    .mysql(TypeSupport::Unsupported("MySQL has no vector type"))
    .sqlite(TypeSupport::Unsupported("SQLite has no vector type"));

const BARE: ColumnSpec = ColumnSpec::new("bare", SqlType::Text);

const UNBOUNDED: ColumnSpec =
    ColumnSpec::new("blob", SqlType::Text).mysql(TypeSupport::named("LONGTEXT"));

fn create(spec: &'static ColumnSpec) -> SchemaOp {
    SchemaOp::CreateTable(
        Table::named("t").column(ColumnDef::new("c", spec.column_type()).unique()),
    )
}

fn unsupported(kind: &'static str, dialect: DialectId, reason: &'static str) -> RenderError {
    RenderError::Schema(Box::new(SchemaError::UnsupportedType {
        column: "c",
        kind,
        dialect,
        reason,
    }))
}

#[test]
fn kinds_render_their_name_with_or_without_arguments() {
    let [citext_pg, citext_my, _] = all(&create(&CITEXT));
    let [vector_pg, ..] = all(&create(&VECTOR));

    assert_eq!(
        citext_pg.as_deref(),
        Ok(r#"CREATE TABLE "t" ("c" CITEXT NOT NULL UNIQUE)"#)
    );
    assert_eq!(
        citext_my.as_deref(),
        Ok("CREATE TABLE `t` (`c` VARCHAR(255) NOT NULL UNIQUE)")
    );
    assert_eq!(
        vector_pg.as_deref(),
        Ok(r#"CREATE TABLE "t" ("c" vector(3, 4) NOT NULL UNIQUE)"#)
    );
}

#[test]
fn a_dialect_without_the_kind_refuses_with_its_reason() {
    let [_, _, citext_sqlite] = all(&create(&CITEXT));
    let [_, vector_mysql, vector_sqlite] = all(&create(&VECTOR));
    let add = SchemaOp::AddColumn {
        table: NOTES,
        column: ColumnDef::new("c", VECTOR.column_type()).nullable(),
    };

    assert_eq!(
        citext_sqlite,
        Err(unsupported(
            "citext",
            DialectId::Sqlite,
            "SQLite has no citext type"
        ))
    );
    assert_eq!(
        vector_mysql,
        Err(unsupported(
            "vector",
            DialectId::MySql,
            "MySQL has no vector type"
        ))
    );
    assert_eq!(
        vector_sqlite,
        Err(unsupported(
            "vector",
            DialectId::Sqlite,
            "SQLite has no vector type"
        ))
    );
    assert_eq!(
        render_schema(&add, &Sqlite),
        Err(unsupported(
            "vector",
            DialectId::Sqlite,
            "SQLite has no vector type"
        ))
    );
}

#[test]
fn kinds_without_a_declared_dialect_are_refused_everywhere() {
    for result in all(&create(&BARE)) {
        assert!(
            matches!(
                result,
                Err(RenderError::Schema(ref e))
                    if matches!(**e, SchemaError::UnsupportedType { kind: "bare", .. })
            ),
            "{result:?}"
        );
    }
}

#[test]
fn mysql_keys_and_indexes_need_a_bounded_kind() {
    let unbounded = UNBOUNDED.column_type();
    let bounded = CITEXT.column_type();
    let table = |ty| {
        SchemaOp::CreateTable(
            Table::named("t")
                .column(ColumnDef::new("c", ty))
                .primary_key(["c"]),
        )
    };
    let indexed = |ty| SchemaOp::CreateIndex(index("i", BTree::METHOD, [("c", ty)]));

    assert!(render_schema(&table(bounded), &MySql).is_ok());
    assert!(render_schema(&indexed(bounded), &MySql).is_ok());
    assert_eq!(
        render_schema(&table(unbounded), &MySql),
        Err(RenderError::Schema(Box::new(SchemaError::Unindexable {
            column: "c",
            ty: unbounded,
            dialect: DialectId::MySql,
        })))
    );
    assert_eq!(
        render_schema(&indexed(unbounded), &MySql),
        Err(refusal(
            "i",
            DialectId::MySql,
            IndexRefusal::Unbounded {
                column: "c",
                ty: unbounded
            }
        ))
    );
}
