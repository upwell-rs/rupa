use super::*;
use crate::column::{Column, ColumnMeta, Scalar, ScalarCodec, StorageKind, column_meta};
use crate::entity::{Entity, FromRow};
use crate::error::ResultError;
use crate::row::Row;
use crate::varchar::VarChar;

/// A hand-written entity with an identity key and a foreign key.
struct Post {
    id: i64,
}

impl Post {
    const TITLE: Column<Post, String, Scalar> = Column::scalar("title");
    const AUTHOR: Column<Post, Option<i64>, Scalar> = Column::scalar("author_id");
    const SLUG: Column<Post, VarChar<40>, Scalar> = Column::scalar("slug");
}

const AUTHORS: Reference = Reference {
    table: TableRef::new("authors"),
    columns: &["id"],
};

impl FromRow for Post {
    fn from_row(_row: &dyn Row) -> Result<Self, ResultError> {
        Ok(Self { id: 0 })
    }
}

impl Entity for Post {
    type Id = i64;
    const TABLE: TableRef = TableRef::with_schema("blog", "posts");
    const ID_COLUMNS: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [ColumnMeta] = &[
        column_meta::<i64, ScalarCodec<i64>>("id", "id")
            .primary_key()
            .generated(Generated::Identity),
        column_meta::<String, ScalarCodec<String>>("title", "title"),
        column_meta::<Option<i64>, ScalarCodec<Option<i64>>>("author", "author_id")
            .references(AUTHORS),
    ];

    fn id(&self) -> &i64 {
        &self.id
    }

    fn to_values(&self) -> Vec<Value> {
        Vec::new()
    }
}

#[test]
fn an_entity_describes_its_table() {
    let table = Table::of::<Post>();

    assert_eq!(table.table, TableRef::with_schema("blog", "posts"));
    assert_eq!(
        table.columns,
        [
            ColumnDef::new("id", ColumnType::BigInt).identity(),
            ColumnDef::new("title", ColumnType::Text),
            ColumnDef::new("author_id", ColumnType::BigInt).nullable(),
        ]
    );
    assert_eq!(table.primary_key, ["id"]);
    assert_eq!(
        table.foreign_keys,
        [ForeignKey::new(["author_id"], AUTHORS)]
    );
    assert_eq!(StorageKind::Scalar, Post::TITLE.kind());
}

#[test]
fn entity_tables_can_be_adjusted() {
    let table = Table::of::<Post>()
        .column(ColumnDef::of(Post::TITLE).ty(ColumnType::VarChar(200)))
        .without("author_id")
        .column(
            ColumnDef::new("draft", ColumnType::Bool)
                .default(ColumnDefault::Value(Value::Bool(true))),
        );

    assert_eq!(
        table
            .columns
            .iter()
            .map(|c| (c.name, c.ty))
            .collect::<Vec<_>>(),
        [
            ("id", ColumnType::BigInt),
            ("title", ColumnType::VarChar(200)),
            ("draft", ColumnType::Bool),
        ]
    );
    assert!(table.foreign_keys.is_empty());
}

#[test]
fn column_handles_describe_their_columns() {
    assert_eq!(
        ColumnDef::of(Post::AUTHOR),
        ColumnDef::new("author_id", ColumnType::BigInt).nullable()
    );
    assert_eq!(
        ForeignKey::to::<Post>(["post_id"])
            .on_delete(ReferentialAction::Cascade)
            .references,
        Reference {
            table: Post::TABLE,
            columns: &["id"],
        }
    );
}

#[test]
fn bounded_text_declares_its_length() {
    assert_eq!(
        column::<VarChar<256>>("email"),
        ColumnDef::new("email", ColumnType::VarChar(256))
    );
    assert_eq!(
        column::<Option<VarChar<16>>>("code"),
        ColumnDef::new("code", ColumnType::VarChar(16)).nullable()
    );
    assert_eq!(
        ColumnDef::of(Post::SLUG),
        ColumnDef::new("slug", ColumnType::VarChar(40))
    );
    assert_eq!(Post::SLUG.meta("slug").column_type, ColumnType::VarChar(40));
    assert_eq!(column::<String>("title").ty, ColumnType::Text);
}
