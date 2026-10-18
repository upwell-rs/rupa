//! Inserts, updates and deletes.

use super::*;

pub async fn case_update<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let meta = fixture::Meta {
        score: 100,
        flag: false,
        tags: vec!["new".into()],
        nested: None,
        note: Some("n".into()),
    };
    let n = e
        .run(
            update::<Item>()
                .set(col!(Item::meta), &meta)
                .set_expr(col!(Item::label), col!(Item::name))
                .filter(col!(Item::active).eq(true))
                .affected(),
        )
        .await
        .unwrap();
    assert_eq!(n, 3);
    let rows = e
        .run(select::<Item>().order_by(col!(Item::id).asc()).all())
        .await
        .unwrap();
    for item in &rows {
        if item.active {
            assert_eq!(item.meta, meta);
            assert_eq!(
                item.label.as_deref(),
                Some(item.name.as_str()),
                "SET sees the pre-update row"
            );
        } else {
            assert_eq!(item, &items()[1]);
        }
    }

    let any = e
        .run(
            update::<Item>()
                .set(col!(Item::qty), &1)
                .filter(col!(Item::id).eq(99))
                .build::<bool>(),
        )
        .await
        .unwrap();
    assert!(!any);
}

pub async fn case_delete<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(
        e.run(delete::<Item>().filter(col!(Item::qty).lt(5)).affected())
            .await
            .unwrap(),
        2
    );
    assert_eq!(ids(e, col!(Item::id).gt(0)).await, [2, 3]);
    assert!(
        !e.run(
            delete::<Item>()
                .filter(col!(Item::id).eq(1))
                .build::<bool>()
        )
        .await
        .unwrap()
    );
    assert_eq!(e.run(delete::<Item>().affected()).await.unwrap(), 2);
}

pub async fn case_duplicate_key_is_an_error<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let r = e.run(insert::<Item>().value(&items()[0]).affected()).await;
    assert!(r.is_err(), "duplicate primary key must fail");
}

pub async fn case_failed_multi_row_insert_is_atomic<E: AsyncExecutor>(e: &mut E) {
    let mut batch = items();
    batch.push(items()[0].clone());
    assert!(
        e.run(insert::<Item>().values(&batch).affected())
            .await
            .is_err()
    );
    assert!(
        !e.run(select::<Item>().exists()).await.unwrap(),
        "no partial insert"
    );
}

/// One model: the entity inserts itself; its generated id is ignored.
///
/// Asking for RETURNING where it does not exist is an error, not a silent no-op.
pub async fn case_generated_ids_and_returning<E: AsyncExecutor>(e: &mut E) {
    let first = insert_notes(e, &[new_note("first")]).await;
    assert_eq!(
        first,
        [Note {
            id: 1,
            title: "first".into(),
            body: None,
            pinned: false
        }]
    );

    let more = insert_notes(e, &[new_note("b"), new_note("c")]).await;
    assert_eq!(more.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 3]);

    let draft = Note {
        id: 0,
        title: "draft".into(),
        body: Some("text".into()),
        pinned: true,
    };
    if returning(e) {
        let saved = e
            .run(insert::<Note>().value(&draft).returning_one())
            .await
            .unwrap();
        assert_eq!(saved, Note { id: 4, ..draft });
    } else {
        e.run(insert::<Note>().value(&draft).affected())
            .await
            .unwrap();
        assert_eq!(
            e.run(get::<Note>(&4)).await.unwrap(),
            Some(Note { id: 4, ..draft })
        );
        let refused = e
            .run(insert::<Note>().value(&new_note("x")).returning_one())
            .await;
        assert!(refused.is_err());
    }
}

pub async fn case_get_by_id<E: AsyncExecutor>(e: &mut E) {
    e.run(
        insert::<Note>()
            .values(&[new_note("a"), new_note("b")])
            .affected(),
    )
    .await
    .unwrap();
    let b = e.run(get::<Note>(&2)).await.unwrap();
    assert_eq!(b.map(|n| n.title), Some("b".to_string()));
    assert_eq!(e.run(get::<Note>(&99)).await.unwrap(), None);
}

/// Full entity, by id.
///
/// Keyed patch: `None` fields stay as they are.
///
/// Unkeyed patch over a filter.
pub async fn case_update_entity_and_patches<E: AsyncExecutor>(e: &mut E) {
    let mut a = insert_notes(e, &[new_note("a")]).await.remove(0);
    e.run(insert::<Note>().value(&new_note("b")).affected())
        .await
        .unwrap();

    a.title = "a2".into();
    a.body = Some("body".into());
    assert_eq!(e.run(update::<Note>().one(&a).affected()).await.unwrap(), 1);
    assert_eq!(e.run(get::<Note>(&1)).await.unwrap(), Some(a.clone()));

    let patch = NotePatch {
        id: 1,
        body: Some(None),
        pinned: None,
    };
    e.run(update::<Note>().one(&patch).affected())
        .await
        .unwrap();
    let got = e.run(get::<Note>(&1)).await.unwrap().unwrap();
    assert_eq!(got, Note { body: None, ..a });

    let n = e
        .run(
            update::<Note>()
                .apply(&SetPinned { pinned: Some(true) })
                .filter(col!(Note::id).gt(0))
                .affected(),
        )
        .await
        .unwrap();
    assert_eq!(n, 2);
    let pinned = e
        .run(select::<Note>().filter(col!(Note::pinned).eq(true)).all())
        .await
        .unwrap();
    assert_eq!(pinned.len(), 2);
}

pub async fn case_delete_one_and_by_id<E: AsyncExecutor>(e: &mut E) {
    let all = insert_notes(e, &[new_note("a"), new_note("b"), new_note("c")]).await;
    assert_eq!(
        e.run(delete::<Note>().one(&all[0]).affected())
            .await
            .unwrap(),
        1
    );
    assert!(
        e.run(delete::<Note>().by_id(&2).build::<bool>())
            .await
            .unwrap()
    );
    assert!(
        !e.run(delete::<Note>().by_id(&2).build::<bool>())
            .await
            .unwrap()
    );
    let left = e.run(select::<Note>().all()).await.unwrap();
    assert_eq!(left.iter().map(|n| n.id).collect::<Vec<_>>(), [3]);
}
