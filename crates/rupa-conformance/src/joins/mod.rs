//! Join cases. Items 1 and 3 have reviews (10 and 11 on item 1, 12 on item 3);
//! items 2 and 4 have none.

mod declared;

use super::*;

pub use declared::*;

pub(super) async fn seed<E: AsyncExecutor>(e: &mut E) {
    insert::<Item>()
        .values(&items())
        .affected()
        .execute(e)
        .await
        .expect("seed items");
    insert::<Review>()
        .values(&reviews())
        .affected()
        .execute(e)
        .await
        .expect("seed reviews");
}

pub(super) fn item(id: i64) -> Item {
    items()
        .into_iter()
        .find(|i| i.id == id)
        .expect("seeded item")
}

pub(super) fn review(id: i64) -> Review {
    reviews()
        .into_iter()
        .find(|r| r.id == id)
        .expect("seeded review")
}

/// `(item id, review id)` pairs, to compare orderings compactly.
pub(super) fn pairs(rows: &[(Item, Option<Review>)]) -> Vec<(i64, Option<i64>)> {
    rows.iter()
        .map(|(i, r)| (i.id, r.as_ref().map(|r| r.id)))
        .collect()
}

/// `on!` builds the same closures: the join condition, and filters with
/// JSON paths, DSL calls and `$variables`.
///
/// `stars >= 3` keeps reviews 10 and 12. apple (score 5) passes on its
/// score, date (no review left, so `comment` is NULL) on `is_null`;
/// cherry doesn't match the pattern, and the DSL call `ci_eq` drops banana.
///
/// `$variables` are borrowed and cloned, so they are still usable.
pub async fn case_join_on_macro<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = select::<Item>()
        .inner_join_on::<Review>(rupa::on!(Review::item_id == Item::id))
        .order_by(|(_, r)| r.col(Review::ID).asc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            (item(1), review(10)),
            (item(1), review(11)),
            (item(3), review(12)),
        ]
    );

    let min_stars = 3;
    let pattern = "%a%".to_string();
    let rows = (select::<Item>()
        .left_join_on::<Review>(rupa::on!(
            Review::item_id == Item::id && Review::stars >= $min_stars
        ))
        .filter(rupa::on!(
            (Item::meta.score > 4 || Review::comment is_null)
                && Item::name like $pattern
                && !ci_eq(Item::name, "BANANA")
        ))
        .order_by(|(i, _)| i.col(Item::ID).asc())
        .all())
    .execute(e)
    .await
    .unwrap();
    assert_eq!(pairs(&rows), [(1, Some(10)), (4, None)]);
    assert_eq!((min_stars, pattern.len()), (3, 3));
}

/// Whole entities on both sides, including JSON and timestamp columns.
pub async fn case_join_inner<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = select::<Item>()
        .inner_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .order_by(|(_, r)| r.col(Review::ID).asc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            (item(1), review(10)),
            (item(1), review(11)),
            (item(3), review(12)),
        ]
    );
}

/// Items without reviews: the unmatched side's id is NULL.
pub async fn case_join_left<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = select::<Item>()
        .left_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .order_by(|(i, _)| i.col(Item::ID).asc())
        .order_by(|(_, r)| r.col(Review::ID).asc())
        .all()
        .execute(e)
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

    let rows = select::<Item>()
        .left_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .filter(|(_, r)| r.col(Review::ID).is_null())
        .order_by(|(i, _)| i.col(Item::ID).asc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(pairs(&rows), [(2, None), (4, None)]);
}

/// Postgres ordering on every backend: descending puts left-join NULLs first.
///
/// Limit and offset apply to joined rows.
pub async fn case_join_filter_and_order_on_joined_columns<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = select::<Item>()
        .inner_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .filter(|(_, r)| r.col(Review::STARS).ge(4))
        .order_by(|(_, r)| r.col(Review::STARS).desc())
        .all()
        .execute(e)
        .await
        .unwrap();
    let ids: Vec<_> = rows.iter().map(|(i, r)| (i.id, r.id)).collect();
    assert_eq!(ids, [(1, 10), (3, 12)]);

    let rows = select::<Item>()
        .left_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .order_by(|(_, r)| r.col(Review::STARS).desc())
        .order_by(|(i, _)| i.col(Item::ID).asc())
        .all()
        .execute(e)
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

    let rows = select::<Item>()
        .left_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .order_by(|(i, _)| i.col(Item::ID).asc())
        .order_by(|(_, r)| r.col(Review::ID).asc())
        .limit(2)
        .offset(1)
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(pairs(&rows), [(1, Some(11)), (2, None)]);
}

/// A self-join: each item with the labelled items that have more stock.
/// Unmatched rows decode as `None` even though `Item` has non-null JSON and
/// timestamp columns, which come back as `NULL`.
pub async fn case_join_self_left_join_decodes_none<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = select::<Item>()
        .left_join_on::<Item>(|(a, b)| {
            b.col(Item::QTY)
                .gt(a.col(Item::QTY))
                .and(b.col(Item::LABEL).is_not_null())
        })
        .order_by(|(a, _)| a.col(Item::ID).asc())
        .order_by(|(_, b)| b.col(Item::ID).asc())
        .all()
        .execute(e)
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
    let rows = select::<Item>()
        .inner_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
        .left_join_on::<Review>(|(_, r, later)| {
            later
                .col(Review::ITEM_ID)
                .eq(r.col(Review::ITEM_ID))
                .and(later.col(Review::ID).gt(r.col(Review::ID)))
        })
        .order_by(|(_, r, _)| r.col(Review::ID).asc())
        .all()
        .execute(e)
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

/// A plain column (no source handle) refers to the first source.
pub async fn case_join_single_row_shapes_and_exists<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let joined = || {
        select::<Item>()
            .inner_join_on::<Review>(|(i, r)| r.col(Review::ITEM_ID).eq(i.col(Item::ID)))
    };

    let one = joined()
        .filter(|(_, r)| r.col(Review::ID).eq(12))
        .one()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(one, (item(3), review(12)));

    let none = joined()
        .filter(|(i, _)| i.col(Item::ID).eq(2))
        .optional()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(none, None);

    let r = joined()
        .filter(|(i, _)| i.col(Item::ID).eq(1))
        .one()
        .execute(e)
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);

    let reviewed = joined()
        .filter(|(_, _)| Item::ID.eq(1))
        .exists()
        .execute(e)
        .await
        .unwrap();
    assert!(reviewed);
    let reviewed = joined()
        .filter(|(i, _)| i.col(Item::ID).eq(4))
        .exists()
        .execute(e)
        .await
        .unwrap();
    assert!(!reviewed);
}
