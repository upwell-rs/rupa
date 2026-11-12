use super::*;

#[test]
fn index_kinds_render_their_method_where_supported() {
    let gin = index("notes_doc", Gin::METHOD, [("doc", ColumnType::Json)]);
    let hash = index("notes_slug", Hash::METHOD, [("slug", ColumnType::Text)]);

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(gin), &Postgres).as_deref(),
        Ok(r#"CREATE INDEX "notes_doc" ON "app"."notes" USING gin ("doc")"#)
    );
    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(hash), &Postgres).as_deref(),
        Ok(r#"CREATE INDEX "notes_slug" ON "app"."notes" USING hash ("slug")"#)
    );
}

#[test]
fn unsupported_kinds_are_refused_with_a_typed_error() {
    let gin = index("notes_doc", Gin::METHOD, [("doc", ColumnType::Json)]);
    let hash = index("notes_slug", Hash::METHOD, [("slug", ColumnType::Text)]);

    for (index, dialect, id, kind, reason) in [
        (
            &gin,
            &MySql as &dyn Dialect,
            DialectId::MySql,
            "gin",
            "MySQL has no GIN indexes",
        ),
        (
            &gin,
            &Sqlite,
            DialectId::Sqlite,
            "gin",
            "SQLite has no GIN indexes",
        ),
        (
            &hash,
            &MySql,
            DialectId::MySql,
            "hash",
            "InnoDB has no hash indexes",
        ),
    ] {
        assert_eq!(
            render_schema(&SchemaOp::CreateIndex(index.clone()), dialect),
            Err(refusal(index.name, id, IndexRefusal::Kind { kind, reason }))
        );
    }
}

#[test]
fn kind_limits_are_checked_at_render() {
    let unique_gin = index("a", Gin::METHOD, [("doc", ColumnType::Json)]).unique();
    let wide_hash = index(
        "b",
        Hash::METHOD,
        [("x", ColumnType::Int), ("y", ColumnType::Int)],
    );

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(unique_gin), &Postgres),
        Err(refusal(
            "a",
            DialectId::Postgres,
            IndexRefusal::Unique { kind: "gin" }
        ))
    );
    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(wide_hash), &Postgres),
        Err(refusal(
            "b",
            DialectId::Postgres,
            IndexRefusal::MultiColumn { kind: "hash" }
        ))
    );
}

#[test]
fn mysql_refuses_unbounded_index_columns_but_others_accept_them() {
    for ty in [ColumnType::Text, ColumnType::Bytes, ColumnType::Json] {
        let op = SchemaOp::CreateIndex(index(
            "notes_body",
            BTree::METHOD,
            [("id", ColumnType::BigInt), ("body", ty)],
        ));

        assert_eq!(
            render_schema(&op, &MySql),
            Err(refusal(
                "notes_body",
                DialectId::MySql,
                IndexRefusal::Unbounded { column: "body", ty }
            ))
        );
        assert!(render_schema(&op, &Postgres).is_ok());
        assert!(render_schema(&op, &Sqlite).is_ok());
    }

    let bounded = SchemaOp::CreateIndex(index(
        "notes_body",
        BTree::METHOD,
        [("body", ColumnType::VarChar(255))],
    ));

    assert_eq!(
        render_schema(&bounded, &MySql).as_deref(),
        Ok("CREATE INDEX `notes_body` ON `app`.`notes` (`body`)")
    );
}
