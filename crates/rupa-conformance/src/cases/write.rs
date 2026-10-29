//! Inserts, updates and deletes.

use super::*;

pub async fn case_update<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let meta = fixture::Meta {
        score: 100,
        flag: false,
        tags: vec!["new".into()],
        nested: None,
        note: Some("n".into()),
    };
    let n = update::<Item>()
        .set(Item::META, &meta)
        .set_expr(Item::LABEL, Item::NAME)
        .filter(Item::ACTIVE.eq(true))
        .affected()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(n, 3);
    let rows = select::<Item>()
        .order_by(Item::ID.asc())
        .all()
        .execute(e)
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

    let any = update::<Item>()
        .set(Item::QTY, &1)
        .filter(Item::ID.eq(99))
        .build::<bool>()
        .execute(e)
        .await
        .unwrap();
    assert!(!any);
}

pub async fn case_delete<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(
        delete::<Item>()
            .filter(Item::QTY.lt(5))
            .affected()
            .execute(e)
            .await
            .unwrap(),
        2
    );
    assert_eq!(ids(e, Item::ID.gt(0)).await, [2, 3]);
    assert!(
        !delete::<Item>()
            .filter(Item::ID.eq(1))
            .build::<bool>()
            .execute(e)
            .await
            .unwrap()
    );
    assert_eq!(delete::<Item>().affected().execute(e).await.unwrap(), 2);
}

pub async fn case_duplicate_key_is_an_error<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let r = insert::<Item>()
        .value(&items()[0])
        .affected()
        .execute(e)
        .await;
    assert!(r.is_err(), "duplicate primary key must fail");
}

pub async fn case_failed_multi_row_insert_is_atomic<E: AsyncExecutor>(e: &E) {
    let mut batch = items();
    batch.push(items()[0].clone());
    assert!(
        insert::<Item>()
            .values(&batch)
            .affected()
            .execute(e)
            .await
            .is_err()
    );
    assert!(
        !select::<Item>().exists().execute(e).await.unwrap(),
        "no partial insert"
    );
}

/// One model: the entity inserts itself; its generated id is ignored.
///
/// Asking for RETURNING where it does not exist is an error, not a silent no-op.
pub async fn case_generated_ids_and_returning<E: AsyncExecutor>(e: &E) {
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
        let saved = insert::<Note>()
            .value(&draft)
            .returning_one()
            .execute(e)
            .await
            .unwrap();
        assert_eq!(saved, Note { id: 4, ..draft });
    } else {
        insert::<Note>()
            .value(&draft)
            .affected()
            .execute(e)
            .await
            .unwrap();
        assert_eq!(
            get::<Note>(&4).execute(e).await.unwrap(),
            Some(Note { id: 4, ..draft })
        );
        let refused = insert::<Note>()
            .value(&new_note("x"))
            .returning_one()
            .execute(e)
            .await;
        assert!(refused.is_err());
    }
}

pub async fn case_get_by_id<E: AsyncExecutor>(e: &E) {
    insert::<Note>()
        .values(&[new_note("a"), new_note("b")])
        .affected()
        .execute(e)
        .await
        .unwrap();
    let b = get::<Note>(&2).execute(e).await.unwrap();
    assert_eq!(b.map(|n| n.title), Some("b".to_string()));
    assert_eq!(get::<Note>(&99).execute(e).await.unwrap(), None);
}

/// Full entity, by id.
///
/// Keyed patch: `None` fields stay as they are.
///
/// Unkeyed patch over a filter.
pub async fn case_update_entity_and_patches<E: AsyncExecutor>(e: &E) {
    let mut a = insert_notes(e, &[new_note("a")]).await.remove(0);
    insert::<Note>()
        .value(&new_note("b"))
        .affected()
        .execute(e)
        .await
        .unwrap();

    a.title = "a2".into();
    a.body = Some("body".into());
    assert_eq!(
        update::<Note>()
            .one(&a)
            .affected()
            .execute(e)
            .await
            .unwrap(),
        1
    );
    assert_eq!(get::<Note>(&1).execute(e).await.unwrap(), Some(a.clone()));

    let patch = NotePatch {
        id: 1,
        body: Some(None),
        pinned: None,
    };
    update::<Note>()
        .one(&patch)
        .affected()
        .execute(e)
        .await
        .unwrap();
    let got = get::<Note>(&1).execute(e).await.unwrap().unwrap();
    assert_eq!(got, Note { body: None, ..a });

    let n = update::<Note>()
        .apply(&SetPinned { pinned: Some(true) })
        .filter(Note::ID.gt(0))
        .affected()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(n, 2);
    let pinned = select::<Note>()
        .filter(Note::PINNED.eq(true))
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(pinned.len(), 2);
}

pub async fn case_delete_one_and_by_id<E: AsyncExecutor>(e: &E) {
    let all = insert_notes(e, &[new_note("a"), new_note("b"), new_note("c")]).await;
    assert_eq!(
        delete::<Note>()
            .one(&all[0])
            .affected()
            .execute(e)
            .await
            .unwrap(),
        1
    );
    assert!(
        delete::<Note>()
            .by_id(&2)
            .build::<bool>()
            .execute(e)
            .await
            .unwrap()
    );
    assert!(
        !delete::<Note>()
            .by_id(&2)
            .build::<bool>()
            .execute(e)
            .await
            .unwrap()
    );
    let left = select::<Note>().all().execute(e).await.unwrap();
    assert_eq!(left.iter().map(|n| n.id).collect::<Vec<_>>(), [3]);
}
