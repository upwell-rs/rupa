use super::*;

#[test]
fn alterations_follow_each_dialects_grammar() {
    let cases = [
        (
            SchemaOp::RenameTable {
                table: NOTES,
                to: "posts",
            },
            [
                r#"ALTER TABLE "app"."notes" RENAME TO "posts""#,
                "ALTER TABLE `app`.`notes` RENAME TO `app`.`posts`",
                r#"ALTER TABLE "app"."notes" RENAME TO "posts""#,
            ],
        ),
        (
            SchemaOp::AddColumn {
                table: NOTES,
                column: ColumnDef::new("slug", ColumnType::Text).nullable(),
            },
            [
                r#"ALTER TABLE "app"."notes" ADD COLUMN "slug" TEXT"#,
                "ALTER TABLE `app`.`notes` ADD COLUMN `slug` LONGTEXT",
                r#"ALTER TABLE "app"."notes" ADD COLUMN "slug" TEXT"#,
            ],
        ),
        (
            SchemaOp::RenameColumn {
                table: NOTES,
                from: "body",
                to: "text",
            },
            [
                r#"ALTER TABLE "app"."notes" RENAME COLUMN "body" TO "text""#,
                "ALTER TABLE `app`.`notes` RENAME COLUMN `body` TO `text`",
                r#"ALTER TABLE "app"."notes" RENAME COLUMN "body" TO "text""#,
            ],
        ),
        (
            SchemaOp::DropColumn {
                table: NOTES,
                column: "body",
            },
            [
                r#"ALTER TABLE "app"."notes" DROP COLUMN "body""#,
                "ALTER TABLE `app`.`notes` DROP COLUMN `body`",
                r#"ALTER TABLE "app"."notes" DROP COLUMN "body""#,
            ],
        ),
        (
            SchemaOp::CreateIndex(
                index(
                    "notes_title",
                    BTree::METHOD,
                    [
                        ("title", ColumnType::VarChar(80)),
                        ("id", ColumnType::BigInt),
                    ],
                )
                .unique(),
            ),
            [
                r#"CREATE UNIQUE INDEX "notes_title" ON "app"."notes" ("title", "id")"#,
                "CREATE UNIQUE INDEX `notes_title` ON `app`.`notes` (`title`, `id`)",
                r#"CREATE UNIQUE INDEX "app"."notes_title" ON "notes" ("title", "id")"#,
            ],
        ),
        (
            SchemaOp::DropIndex {
                table: NOTES,
                name: "notes_title",
            },
            [
                r#"DROP INDEX "app"."notes_title""#,
                "DROP INDEX `notes_title` ON `app`.`notes`",
                r#"DROP INDEX "app"."notes_title""#,
            ],
        ),
        (
            SchemaOp::DropTable(NOTES),
            [
                r#"DROP TABLE "app"."notes""#,
                "DROP TABLE `app`.`notes`",
                r#"DROP TABLE "app"."notes""#,
            ],
        ),
    ];

    for (op, expected) in cases {
        assert_eq!(ok(&op), expected, "{op:?}");
    }
}

#[test]
fn defaults_are_checked_literals_or_parenthesized_expressions() {
    let column = |ty, default| SchemaOp::AddColumn {
        table: NOTES,
        column: ColumnDef::new("c", ty).default(default),
    };
    let [postgres, mysql, sqlite] = ok(&column(
        ColumnType::Text,
        ColumnDefault::Value(Value::Text("it's".into())),
    ));
    let [timestamp, ..] = ok(&column(
        ColumnType::TimestampTz,
        ColumnDefault::Sql("CURRENT_TIMESTAMP"),
    ));

    assert!(
        postgres.ends_with(r#""c" TEXT NOT NULL DEFAULT 'it''s'"#),
        "{postgres}"
    );
    assert!(
        mysql.ends_with("`c` LONGTEXT NOT NULL DEFAULT ('it''s')"),
        "{mysql}"
    );
    assert!(
        sqlite.ends_with(r#""c" TEXT NOT NULL DEFAULT 'it''s'"#),
        "{sqlite}"
    );
    assert!(
        timestamp.ends_with("DEFAULT (CURRENT_TIMESTAMP)"),
        "{timestamp}"
    );
    assert!(
        render_schema(
            &column(
                ColumnType::Text,
                ColumnDefault::Value(Value::Text("a\\b'c".into()))
            ),
            &Postgres
        )
        .expect("an escape string")
        .ends_with(r"DEFAULT E'a\\b''c'"),
        "PostgreSQL backslashes mean the same whatever standard_conforming_strings is"
    );
    assert_eq!(
        render_schema(
            &column(
                ColumnType::Text,
                ColumnDefault::Value(Value::Text("a\\b".into()))
            ),
            &MySql
        ),
        Err(RenderError::Schema(Box::new(SchemaError::Default {
            column: "c",
            dialect: DialectId::MySql
        })))
    );
}
