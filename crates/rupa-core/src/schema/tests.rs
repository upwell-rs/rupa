use super::*;
use crate::column::{Column, ColumnKind, ColumnMeta, Scalar, ScalarCodec, column_meta};
use crate::entity::{Entity, FromRow};
use crate::error::ResultError;
use crate::row::Row;

/// A hand-written entity with an identity key and a foreign key.
struct Post {
    id: i64,
}

impl Post {
    const TITLE: Column<Post, String, Scalar> = Column::scalar("title");
    const AUTHOR: Column<Post, Option<i64>, Scalar> = Column::scalar("author_id");
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
    assert_eq!(ColumnKind::Scalar, Post::TITLE.kind());
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
fn column_handles_and_indexes_name_their_entity() {
    assert_eq!(
        ColumnDef::of(Post::AUTHOR),
        ColumnDef::new("author_id", ColumnType::BigInt).nullable()
    );
    assert_eq!(
        Index::on("posts_title", Post::TITLE).unique(),
        Index {
            name: "posts_title",
            table: Post::TABLE,
            columns: vec!["title"],
            unique: true,
        }
    );
    assert_eq!(
        Index::of::<Post>("posts_author_title", ["author_id", "title"]).columns,
        ["author_id", "title"]
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
