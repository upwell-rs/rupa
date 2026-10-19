use rupa_core::ir::TableRef;
use rupa_core::schema::{
    ColumnDef, ColumnDefault, ColumnType, ForeignKey, Index, ReferentialAction, SchemaOp, Table,
};
use rupa_core::{Reference, Value};
use rupa_sql::SchemaError;

use super::*;

const NOTES: TableRef = TableRef::new("notes");

/// Planned Rust migrations declare no inputs: their plan is their identity.
fn planless() -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        inputs: &[],
        ..*metadata("1", "")
    }))
}

fn notes() -> Table {
    Table::at(NOTES)
        .column(ColumnDef::new("id", ColumnType::BigInt).identity())
        .column(ColumnDef::new("title", ColumnType::Text))
        .column(ColumnDef::new("author_id", ColumnType::BigInt).nullable())
        .primary_key(["id"])
        .foreign_key(ForeignKey::new(
            ["author_id"],
            Reference {
                table: TableRef::new("authors"),
                columns: &["id"],
            },
        ))
}

#[test]
fn a_planned_migrations_identity_is_its_plan() {
    let first = digest(Planned(Fixed(planless(), || {
        Plan::new().create_table(notes())
    })));
    let refactored = digest(Planned(Fixed(planless(), || {
        Plan::new().schema(SchemaOp::CreateTable(notes()))
    })));

    assert_eq!(
        first.expect("planned migrations need no inputs"),
        refactored.expect("same plan"),
        "how a plan is built is not part of its identity"
    );
    assert!(matches!(
        digest(Fixed(planless(), Plan::new)),
        Err(ValidationError::NoInputs(_))
    ));
}

#[test]
fn every_schema_detail_is_checksummed() {
    let variants: [fn() -> Plan; 10] = [
        || Plan::new().create_table(notes()),
        || {
            Plan::new()
                .create_table(notes().column(ColumnDef::new("title", ColumnType::Text).nullable()))
        },
        || {
            Plan::new()
                .create_table(notes().column(ColumnDef::new("title", ColumnType::VarChar(80))))
        },
        || {
            Plan::new().create_table(
                notes().column(
                    ColumnDef::new("title", ColumnType::Text)
                        .default(ColumnDefault::Value(Value::Text(String::new()))),
                ),
            )
        },
        || {
            Plan::new()
                .create_table(notes().column(ColumnDef::new("title", ColumnType::Text).unique()))
        },
        || {
            Plan::new().create_table(
                notes().foreign_key(
                    ForeignKey::new(
                        ["author_id"],
                        Reference {
                            table: TableRef::new("authors"),
                            columns: &["id"],
                        },
                    )
                    .on_delete(ReferentialAction::Cascade),
                ),
            )
        },
        || {
            Plan::new().create_table(Table {
                table: TableRef::with_schema("main", "notes"),
                ..notes()
            })
        },
        || Plan::new().create_index(Index::new("notes_title", NOTES, ["title"])),
        || Plan::new().create_index(Index::new("notes_title", NOTES, ["title"]).unique()),
        || Plan::new().rename_column(NOTES, "title", "name"),
    ];
    let digests: Vec<_> = variants
        .iter()
        .map(|plan| digest(Planned(Fixed(planless(), *plan))).expect("valid plan"))
        .collect();

    for (i, a) in digests.iter().enumerate() {
        for b in &digests[i + 1..] {
            assert_ne!(a, b, "plan variants must not share a checksum");
        }
    }
}

/// Changing this value changes every applied schema checksum: it needs a new
/// `CHECKSUM_FORMAT`, never an updated expectation.
#[test]
fn schema_encoding_is_stable() {
    let checksum = digest(Planned(Fixed(planless(), || {
        Plan::new()
            .create_table(notes())
            .add_column_to(
                NOTES,
                ColumnDef::new("pinned", ColumnType::Bool)
                    .default(ColumnDefault::Value(Value::Bool(false))),
            )
            .create_index(Index::new("notes_title", NOTES, ["title"]).unique())
    })));

    assert_eq!(
        checksum.expect("valid plan").to_string(),
        "8afc2b08c1a29c96f53f3a9add99613ae39893d819c71e36fd914dce71070ac8"
    );
}

#[test]
fn plans_are_validated_for_the_session_dialect() {
    let migrations = [Planned(Fixed(planless(), || {
        Plan::new().add_column_to(NOTES, ColumnDef::new("slug", ColumnType::Text))
    }))];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &migrations).expect("valid catalog");

    assert!(catalog.validate_plans(DialectId::Postgres).is_ok());
    assert!(matches!(
        catalog.validate_plans(DialectId::Sqlite),
        Err(ValidationError::Schema(error)) if matches!(
            *error,
            RenderError::Schema(ref schema)
                if matches!(**schema, SchemaError::AddColumn { column: "slug", .. })
        )
    ));
    assert!(matches!(
        catalog.validate_plans(DialectId::Memory),
        Err(ValidationError::Schema(error))
            if *error == RenderError::UnsupportedDialect(DialectId::Memory)
    ));
}
