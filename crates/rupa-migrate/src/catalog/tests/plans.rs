use rupa_core::ir::TableRef;
use rupa_core::schema::{
    ColumnDef, ColumnDefault, ColumnSpec, ColumnType, ForeignKey, Gin, Index, IndexColumn,
    IndexKind, IndexMethod, IndexSupport, ReferentialAction, SchemaOp, Table, TypeSupport,
};
use rupa_core::{Reference, SqlType, Value};
use rupa_sql::SchemaError;
use sha2::{Digest, Sha256};

use super::*;

pub(super) const NOTES: TableRef = TableRef::new("notes");

/// Planned Rust migrations declare no inputs: their plan is their identity.
pub(super) fn planless() -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        inputs: &[],
        ..*metadata("1", "")
    }))
}

pub(super) fn notes() -> Table {
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

pub(super) fn notes_title() -> Index {
    Index::new(
        "notes_title",
        NOTES,
        [IndexColumn::new("title", ColumnType::Text)],
    )
}

pub(super) fn plan_digest(plan: fn() -> Plan) -> Checksum {
    digest(Planned(Fixed(planless(), plan))).expect("valid plan")
}

pub(super) fn assert_distinct_digests(variants: &[fn() -> Plan], message: &str) {
    let digests: Vec<_> = variants.iter().map(|plan| plan_digest(*plan)).collect();

    for (i, a) in digests.iter().enumerate() {
        for b in &digests[i + 1..] {
            assert_ne!(a, b, "{message}");
        }
    }
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
    let variants: [fn() -> Plan; 11] = [
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
        || Plan::new().create_index(notes_title()),
        || Plan::new().create_index(notes_title().unique()),
        || Plan::new().create_index(notes_title().method(Gin::METHOD)),
        || Plan::new().rename_column(NOTES, "title", "name"),
    ];

    assert_distinct_digests(&variants, "plan variants must not share a checksum");
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
            .create_index(notes_title().unique())
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

    assert!(
        catalog
            .check(DialectId::Postgres, &[Policy::TransactionalRequired])
            .is_ok()
    );
    assert!(matches!(
        catalog.check(DialectId::Sqlite, &[Policy::TransactionalRequired]),
        Err(ValidationError::Schema(error)) if matches!(
            *error,
            RenderError::Schema(ref schema)
                if matches!(**schema, SchemaError::AddColumn { column: "slug", .. })
        )
    ));
    assert!(matches!(
        catalog.check(DialectId::Memory, &[Policy::TransactionalRequired]),
        Err(ValidationError::Schema(error))
            if *error == RenderError::UnsupportedDialect(DialectId::Memory)
    ));
}

#[test]
fn an_empty_plan_is_rejected() {
    assert!(matches!(
        digest(Planned(Fixed(planless(), Plan::new))),
        Err(ValidationError::EmptyPlan(version)) if version == "1"
    ));
}

#[test]
fn json_defaults_are_checksummed_canonically() {
    let mut first = serde_json::Map::new();
    let mut second = serde_json::Map::new();

    first.insert("a".into(), serde_json::json!(1));
    first.insert("b".into(), serde_json::json!([true, null, 2.5]));
    second.insert("b".into(), serde_json::json!([true, null, 2.5]));
    second.insert("a".into(), serde_json::json!(1));

    let encode = |json: serde_json::Map<String, serde_json::Value>| {
        let mut hash = Sha256::new();

        encode::Encoder(&mut hash).schema(&SchemaOp::AddColumn {
            table: NOTES,
            column: ColumnDef::new("settings", ColumnType::Json)
                .default(ColumnDefault::Value(Value::Json(json.into()))),
        });

        hash.finalize()
    };

    assert_eq!(
        encode(first),
        encode(second),
        "key order is not part of a JSON default's identity"
    );
}

const CUSTOM: IndexMethod = IndexMethod::new("custom").postgres(IndexSupport::Using("hash"));

#[test]
fn an_index_methods_rendering_configuration_is_part_of_its_checksum() {
    let variants: [fn() -> Plan; 6] = [
        || Plan::new().create_index(notes_title().method(CUSTOM)),
        || {
            Plan::new().create_index(
                notes_title()
                    .method(IndexMethod::new("custom").postgres(IndexSupport::Using("gin"))),
            )
        },
        || {
            Plan::new().create_index(
                notes_title().method(IndexMethod::new("custom").postgres(IndexSupport::Default)),
            )
        },
        || Plan::new().create_index(notes_title().method(CUSTOM.supports_unique())),
        || Plan::new().create_index(notes_title().method(CUSTOM.single_column())),
        || {
            Plan::new()
                .create_index(notes_title().method(CUSTOM.mysql(IndexSupport::Using("BTREE"))))
        },
    ];

    assert_distinct_digests(&variants, "every rendering setting is part of the identity");
}

const KIND: ColumnSpec = ColumnSpec::new("shape", SqlType::Text)
    .postgres(TypeSupport::with_args("shape", &[3]))
    .mysql(TypeSupport::named("LONGTEXT"))
    .sqlite(TypeSupport::Unsupported("SQLite has no shape type"));

fn with_kind(spec: ColumnSpec) -> Plan {
    let spec = Box::leak(Box::new(spec));

    Plan::new().create_table(notes().column(ColumnDef::new("title", spec.column_type())))
}

#[test]
fn a_column_kinds_rendering_configuration_is_part_of_its_checksum() {
    let variants: [fn() -> Plan; 11] = [
        || with_kind(KIND),
        || {
            with_kind(ColumnSpec {
                name: "form",
                ..KIND
            })
        },
        || with_kind(KIND.postgres(TypeSupport::with_args("shape", &[4]))),
        || with_kind(KIND.postgres(TypeSupport::with_args("shape", &[3, 4]))),
        || with_kind(KIND.postgres(TypeSupport::with_args("shapes", &[3]))),
        || with_kind(KIND.postgres(TypeSupport::named("shape"))),
        || with_kind(KIND.postgres(TypeSupport::Unsupported("shape"))),
        || with_kind(KIND.mysql(TypeSupport::named("TEXT"))),
        || with_kind(KIND.sqlite(TypeSupport::named("TEXT"))),
        || {
            with_kind(ColumnSpec {
                sql_type: SqlType::Json,
                ..KIND
            })
        },
        || with_kind(KIND.bounded()),
    ];

    assert_distinct_digests(&variants, "every rendering setting is part of the identity");
}

#[test]
fn rewording_an_unsupported_reason_keeps_the_checksum() {
    let kinds: [fn() -> Plan; 2] = [
        || with_kind(KIND.sqlite(TypeSupport::Unsupported("SQLite has no shapes"))),
        || with_kind(KIND.sqlite(TypeSupport::Unsupported("shapes need an extension"))),
    ];
    let methods: [fn() -> Plan; 2] = [
        || {
            Plan::new()
                .create_index(notes_title().method(CUSTOM.mysql(IndexSupport::Unsupported("a"))))
        },
        || {
            Plan::new()
                .create_index(notes_title().method(CUSTOM.mysql(IndexSupport::Unsupported("b"))))
        },
    ];

    assert_eq!(plan_digest(kinds[0]), plan_digest(kinds[1]));
    assert_eq!(plan_digest(methods[0]), plan_digest(methods[1]));
}

/// Pins the encodings that `schema_encoding_is_stable` cannot reach: a
/// non-B-tree index method and a column kind. Changing this value changes
/// applied checksums: it needs a new `CHECKSUM_FORMAT`, never an updated
/// expectation.
#[test]
fn index_method_and_column_kind_encoding_is_stable() {
    let checksum = plan_digest(|| with_kind(KIND).create_index(notes_title().method(Gin::METHOD)));

    assert_eq!(
        checksum.to_string(),
        "c9eb36a3cc58a891fa32d164acd735725ba53e04624be1b1286fe4074cd1d311"
    );
}
