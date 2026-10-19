//! Joins from declarations: relations, `#[derive(Joined)]` and `#[query]`.

use super::*;

/// `#[column(references = Item)]` on `Review::item_id`: joins in either
/// direction without a condition, onto whichever source the relation fits.
///
/// Review joined onto Item (`Review.item_id = Item.id`).
///
/// The reverse: Item joined onto Review, naming the relation.
///
/// The relation can be to any earlier source, here the second.
pub async fn case_join_declared_relations<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;

    let rows = e
        .run(
            select::<Item>()
                .join::<Review>()
                .left()
                .order_by(|(i, _)| i.col(Item::ID).asc())
                .order_by(|(_, r)| r.col(Review::ID).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        pairs(&rows),
        [
            (1, Some(10)),
            (1, Some(11)),
            (2, None),
            (3, Some(12)),
            (4, None)
        ]
    );

    let rows = e
        .run(
            select::<Review>()
                .join::<Item>()
                .inner_via(Review::item())
                .order_by(|(r, _)| r.col(Review::ID).desc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            (review(12), item(3)),
            (review(11), item(1)),
            (review(10), item(1)),
        ]
    );

    let notes = [
        NewNote {
            title: "cherry".into(),
            pinned: true,
        },
        NewNote {
            title: "kiwi".into(),
            pinned: false,
        },
    ];
    e.run(insert::<Note>().values(&notes).affected())
        .await
        .unwrap();
    let rows = e
        .run(
            select::<Note>()
                .inner_join_on::<Item>(rupa::on!(Item::name == Note::title))
                .join::<Review>()
                .inner()
                .all(),
        )
        .await
        .unwrap();
    let got: Vec<_> = rows
        .iter()
        .map(|(n, i, r)| (n.title.as_str(), i.id, r.id))
        .collect();
    assert_eq!(got, [("cherry", 3, 12)]);
}

/// One merged row per item and review, instead of a tuple.
#[derive(Debug, PartialEq, rupa::Joined)]
pub struct ItemReview {
    #[from(Item::name)]
    item: String,
    #[from(Item::meta)]
    meta: crate::fixture::Meta,
    /// Left-joined, so an `Option` even though `stars` is `NOT NULL`.
    #[from(Review::stars)]
    stars: Option<i32>,
    /// A nullable column of a left-joined entity: `NULL` either way.
    #[from(Review::comment)]
    comment: Option<String>,
    #[flatten]
    review: Option<Review>,
}

/// Inner-joined fields can be plain.
#[derive(Debug, PartialEq, rupa::Joined)]
pub struct Reviewed {
    #[flatten]
    item: Item,
    #[from(Review::stars)]
    stars: i32,
}

pub async fn case_join_into_merged_struct<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e
        .run(
            select::<Item>()
                .join::<Review>()
                .left()
                .order_by(|(i, _)| i.col(Item::ID).asc())
                .order_by(|(_, r)| r.col(Review::ID).asc())
                .into::<ItemReview>()
                .all(),
        )
        .await
        .unwrap();
    let expected = |id: i64, r: Option<i64>| {
        let (item, review) = (item(id), r.map(review));
        ItemReview {
            item: item.name,
            meta: item.meta,
            stars: review.as_ref().map(|r| r.stars),
            comment: review.as_ref().and_then(|r| r.comment.clone()),
            review,
        }
    };
    assert_eq!(
        rows,
        [
            expected(1, Some(10)),
            expected(1, Some(11)),
            expected(2, None),
            expected(3, Some(12)),
            expected(4, None),
        ]
    );

    let one = e
        .run(
            select::<Item>()
                .join::<Review>()
                .inner()
                .filter(rupa::on!(Review::id == 12))
                .into::<Reviewed>()
                .one(),
        )
        .await
        .unwrap();
    assert_eq!(
        one,
        Reviewed {
            item: item(3),
            stars: 4
        }
    );
}

#[rupa::query(inner_join = Review, filter = Review::stars >= $min, order_by = Review::id)]
fn reviewed(min: i32) -> Vec<(Item, Review)>;

#[rupa::query(
    entity = Item,
    left_join = Review on Review::item_id == Item::id && Review::comment is_null,
    order_by = Item::id,
)]
fn uncommented() -> Vec<ItemReview>;

/// Joins declared with `#[query]`.
pub async fn case_join_query_attribute<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e.run(reviewed(4)).await.unwrap();
    assert_eq!(rows, [(item(1), review(10)), (item(3), review(12))]);

    let rows = e.run(uncommented()).await.unwrap();
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.item.as_str(), r.review.as_ref().map(|r| r.id)))
        .collect();
    assert_eq!(
        got,
        [
            ("apple", Some(11)),
            ("banana", None),
            ("cherry", None),
            ("date", None)
        ]
    );
}

#[rupa::query(
    entity = Item as a,
    left_join = Item as b on b::qty > a::qty && !(b::label is_null),
    order_by = a::id,
    order_by = b::id,
)]
fn more_stock() -> Vec<(Item, Option<Item>)>;

/// A self-join into a struct: fields pinned to their source.
#[derive(Debug, PartialEq, rupa::Joined)]
pub struct StockPair {
    #[from(Item::name, source = 0)]
    name: String,
    #[from(Item::name, source = 1)]
    more: Option<String>,
}

#[rupa::query(
    entity = Item,
    left_join = Item as b on b::qty > Item::qty && !(b::label is_null),
    order_by = Item::id,
    order_by = b::id,
)]
fn stock_pairs() -> Vec<StockPair>;

/// The self-join of `case_join_self_left_join_decodes_none`, through aliases.
pub async fn case_join_self_join_aliases<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let rows = e.run(more_stock()).await.unwrap();
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

    let pairs = e.run(stock_pairs()).await.unwrap();
    let pair = |name: &str, more: Option<&str>| StockPair {
        name: name.into(),
        more: more.map(Into::into),
    };
    assert_eq!(
        pairs,
        [
            pair("apple", Some("cherry")),
            pair("banana", None),
            pair("cherry", None),
            pair("date", Some("apple")),
            pair("date", Some("cherry")),
        ]
    );
}
