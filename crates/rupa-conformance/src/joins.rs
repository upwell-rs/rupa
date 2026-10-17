//! Join cases. Items 1 and 3 have reviews (10 and 11 on item 1, 12 on item 3);
//! items 2 and 4 have none.

use super::*;

async fn seed<E: AsyncExecutor>(e: &mut E) {
    e.run(insert::<Item>().values(&items()).affected())
        .await
        .expect("seed items");
    e.run(insert::<Review>().values(&reviews()).affected())
        .await
        .expect("seed reviews");
}

fn item(id: i64) -> Item {
    items()
        .into_iter()
        .find(|i| i.id == id)
        .expect("seeded item")
}

fn review(id: i64) -> Review {
    reviews()
        .into_iter()
        .find(|r| r.id == id)
        .expect("seeded review")
}

/// `(item id, review id)` pairs, to compare orderings compactly.
fn pairs(rows: &[(Item, Option<Review>)]) -> Vec<(i64, Option<i64>)> {
    rows.iter()
        .map(|(i, r)| (i.id, r.as_ref().map(|r| r.id)))
        .collect()
}

pub async fn case_join_inner<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .inner_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .order_by(|_, r| r.col(col!(Review::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    // Whole entities on both sides, including JSON and timestamp columns.
    assert_eq!(
        rows,
        [
            (item(1), review(10)),
            (item(1), review(11)),
            (item(3), review(12)),
        ]
    );
}

pub async fn case_join_left<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .left_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .order_by(|i, _| i.col(col!(Item::id)).asc())
                .order_by(|_, r| r.col(col!(Review::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            (item(1), Some(review(10))),
            (item(1), Some(review(11))),
            (item(2), None),
            (item(3), Some(review(12))),
            (item(4), None),
        ]
    );

    // Items without reviews: the unmatched side's id is NULL.
    let rows = e
        .run(
            select::<Item>()
                .left_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .filter(|_, r| r.col(col!(Review::id)).is_null())
                .order_by(|i, _| i.col(col!(Item::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(pairs(&rows), [(2, None), (4, None)]);
}

pub async fn case_join_filter_and_order_on_joined_columns<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .inner_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .filter(|_, r| r.col(col!(Review::stars)).ge(4))
                .order_by(|_, r| r.col(col!(Review::stars)).desc())
                .all(),
        )
        .await
        .unwrap();
    let ids: Vec<_> = rows.iter().map(|(i, r)| (i.id, r.id)).collect();
    assert_eq!(ids, [(1, 10), (3, 12)]);

    // Postgres ordering on every backend: descending puts left-join NULLs first.
    let rows = e
        .run(
            select::<Item>()
                .left_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .order_by(|_, r| r.col(col!(Review::stars)).desc())
                .order_by(|i, _| i.col(col!(Item::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        pairs(&rows),
        [
            (2, None),
            (4, None),
            (1, Some(10)),
            (3, Some(12)),
            (1, Some(11)),
        ]
    );

    // Limit and offset apply to joined rows.
    let rows = e
        .run(
            select::<Item>()
                .left_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .order_by(|i, _| i.col(col!(Item::id)).asc())
                .order_by(|_, r| r.col(col!(Review::id)).asc())
                .limit(2)
                .offset(1)
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(pairs(&rows), [(1, Some(11)), (2, None)]);
}

/// A self-join: each item with the labelled items that have more stock.
/// Unmatched rows decode as `None` even though `Item` has non-null JSON and
/// timestamp columns, which come back as `NULL`.
pub async fn case_join_self_left_join_decodes_none<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .left_join::<Item>(|a, b| {
                    b.col(col!(Item::qty))
                        .gt(a.col(col!(Item::qty)))
                        .and(b.col(col!(Item::label)).is_not_null())
                })
                .order_by(|a, _| a.col(col!(Item::id)).asc())
                .order_by(|_, b| b.col(col!(Item::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    let ids: Vec<_> = rows
        .iter()
        .map(|(a, b)| (a.id, b.as_ref().map(|b| b.id)))
        .collect();
    assert_eq!(
        ids,
        [
            (1, Some(3)),
            (2, None),
            (3, None),
            (4, Some(1)),
            (4, Some(3))
        ]
    );
    assert_eq!(rows[0].1, Some(item(3)));
}

/// The third source's condition refers to the second: each review with a
/// later review of the same item.
pub async fn case_join_three_sources<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .inner_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
                .left_join::<Review>(|_, r, later| {
                    later
                        .col(col!(Review::item_id))
                        .eq(r.col(col!(Review::item_id)))
                        .and(later.col(col!(Review::id)).gt(r.col(col!(Review::id))))
                })
                .order_by(|_, r, _| r.col(col!(Review::id)).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            (item(1), review(10), Some(review(11))),
            (item(1), review(11), None),
            (item(3), review(12), None),
        ]
    );
}

pub async fn case_join_single_row_shapes_and_exists<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let joined = || {
        select::<Item>()
            .inner_join::<Review>(|i, r| r.col(col!(Review::item_id)).eq(i.col(col!(Item::id))))
    };

    let one = e
        .run(joined().filter(|_, r| r.col(col!(Review::id)).eq(12)).one())
        .await
        .unwrap();
    assert_eq!(one, (item(3), review(12)));

    let none = e
        .run(
            joined()
                .filter(|i, _| i.col(col!(Item::id)).eq(2))
                .optional(),
        )
        .await
        .unwrap();
    assert_eq!(none, None);

    let r = e
        .run(joined().filter(|i, _| i.col(col!(Item::id)).eq(1)).one())
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);

    // A plain column (no source handle) refers to the first source.
    let reviewed = e
        .run(joined().filter(|_, _| col!(Item::id).eq(1)).exists())
        .await
        .unwrap();
    assert!(reviewed);
    let reviewed = e
        .run(joined().filter(|i, _| i.col(col!(Item::id)).eq(4)).exists())
        .await
        .unwrap();
    assert!(!reviewed);
}
