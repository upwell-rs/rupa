//! Selects: filters, ordering, paging, JSON and timestamps.

use super::*;

pub async fn case_insert_and_select<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let all = select::<Item>()
        .order_by(Item::ID.asc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(all, items(), "rows must round-trip exactly");
}

pub async fn case_single_row_shapes<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let one = select::<Item>()
        .filter(Item::ID.eq(2))
        .one()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(one.name, "banana");
    let none = select::<Item>()
        .filter(Item::ID.eq(99))
        .optional()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(none, None);
    let some = select::<Item>()
        .filter(Item::ID.eq(3))
        .optional()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(some.map(|i| i.id), Some(3));

    let r = select::<Item>()
        .filter(Item::ID.eq(99))
        .one()
        .execute(e)
        .await;
    assert_eq!(result_error(r), ResultError::NotFound);
    let r = select::<Item>()
        .filter(Item::ACTIVE.eq(true))
        .one()
        .execute(e)
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);
    let r = select::<Item>()
        .filter(Item::ACTIVE.eq(true))
        .optional()
        .execute(e)
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);
}

pub async fn case_comparisons<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(ids(e, Item::QTY.eq(7)).await, [3]);
    assert_eq!(ids(e, Item::QTY.ne(7)).await, [1, 2, 4]);
    assert_eq!(ids(e, Item::QTY.lt(7)).await, [1, 4]);
    assert_eq!(ids(e, Item::QTY.le(7)).await, [1, 3, 4]);
    assert_eq!(ids(e, Item::QTY.gt(3)).await, [2, 3]);
    assert_eq!(ids(e, Item::QTY.ge(3)).await, [1, 2, 3]);
    assert_eq!(ids(e, Item::NAME.gt("banana")).await, [3, 4]);
    assert_eq!(
        ids(e, Item::ACTIVE.eq(false).or(Item::QTY.eq(0))).await,
        [2, 4]
    );
}

/// NULL = 'red' is NULL, and NOT NULL is still NULL: neither keeps rows 2 and 4.
///
/// NULL OR TRUE is TRUE; NULL AND FALSE is FALSE.
pub async fn case_three_valued_logic<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(ids(e, Item::LABEL.eq("red")).await, [1]);
    assert_eq!(ids(e, !Item::LABEL.eq("red")).await, [3]);
    assert_eq!(ids(e, Item::LABEL.is_null()).await, [2, 4]);
    assert_eq!(ids(e, Item::LABEL.is_not_null()).await, [1, 3]);
    assert_eq!(ids(e, Item::LABEL.eq("x").or(Item::QTY.eq(12))).await, [2]);
    assert_eq!(
        ids(e, !(Item::LABEL.eq("x").and(Item::QTY.eq(-5)))).await,
        [1, 2, 3, 4]
    );
}

/// x NOT IN (1, NULL) is never TRUE.
pub async fn case_in_lists<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(ids(e, Item::ID.in_([1i64, 3, 99])).await, [1, 3]);
    assert_eq!(ids(e, Item::ID.not_in([1i64, 3])).await, [2, 4]);
    assert_eq!(
        ids(e, Item::ID.in_(Vec::<i64>::new())).await,
        Vec::<i64>::new()
    );
    assert_eq!(
        ids(e, Item::ID.not_in(Vec::<i64>::new())).await,
        [1, 2, 3, 4]
    );
    let null = Expr::<Option<i64>>::from_node(ExprNode::Param(Value::Null(rupa::SqlType::I64)));
    assert_eq!(
        ids(
            e,
            Item::ID.not_in([Expr::from_node(ExprNode::Param(Value::I64(1))), null])
        )
        .await,
        Vec::<i64>::new()
    );
}

pub async fn case_like_patterns<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(ids(e, Item::NAME.like("%an%")).await, [2]);
    assert_eq!(ids(e, Item::NAME.like("_pple")).await, [1]);
    assert_eq!(ids(e, Item::LABEL.like("dark\\_%")).await, [3]);
    assert_eq!(ids(e, Item::LABEL.like("%")).await, [1, 3]);
}

pub async fn case_order_with_nulls<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let asc = select::<Item>()
        .order_by(Item::LABEL.asc())
        .order_by(Item::ID.asc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(
        asc.iter().map(|i| i.id).collect::<Vec<_>>(),
        [3, 1, 2, 4],
        "NULLS LAST ascending"
    );
    let desc = select::<Item>()
        .order_by(Item::LABEL.desc())
        .order_by(Item::ID.desc())
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(
        desc.iter().map(|i| i.id).collect::<Vec<_>>(),
        [4, 2, 1, 3],
        "NULLS FIRST descending"
    );
}

pub async fn case_limit_offset<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let page = select::<Item>()
        .order_by(Item::ID.asc())
        .limit(2)
        .offset(1)
        .all()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(page.iter().map(|i| i.id).collect::<Vec<_>>(), [2, 3]);
    let past_end = select::<Item>()
        .order_by(Item::ID.asc())
        .offset(10)
        .all()
        .execute(e)
        .await
        .unwrap();
    assert!(past_end.is_empty());
    let zero = select::<Item>().limit(0).all().execute(e).await.unwrap();
    assert!(zero.is_empty());
}

pub async fn case_exists<E: AsyncExecutor>(e: &E) {
    assert!(
        !select::<Item>().exists().execute(e).await.unwrap(),
        "empty table"
    );
    seed(e).await;
    assert!(
        select::<Item>()
            .filter(Item::NAME.eq("cherry"))
            .exists()
            .execute(e)
            .await
            .unwrap()
    );
    assert!(
        !select::<Item>()
            .filter(Item::NAME.eq("fig"))
            .exists()
            .execute(e)
            .await
            .unwrap()
    );
}

/// A missing key and a JSON null are both SQL NULL under ->>.
///
/// ...but under ->, a JSON null is a value, not SQL NULL.
pub async fn case_json_paths<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(
        ids(e, Item::META.path("nested").key("k").text().eq("w")).await,
        [3]
    );
    assert_eq!(
        ids(e, Item::META.path("tags").index(0).text().eq("a")).await,
        [1]
    );
    assert_eq!(
        ids(e, Item::META.path("flag").cast::<bool>().eq(true)).await,
        [1, 3]
    );
    assert_eq!(
        ids(e, Item::META.path("score").cast::<i64>().gt(6)).await,
        [2, 3]
    );
    assert_eq!(
        ids(e, Item::META.path("missing").text().is_null()).await,
        [1, 2, 3, 4]
    );
    assert_eq!(
        ids(e, Item::META.path("note").text().is_null()).await,
        [1, 2, 3, 4]
    );
    assert_eq!(
        ids(e, Item::META.path("nested").key("k").text().is_null()).await,
        [1, 4]
    );
    let note_json = Expr::<bool>::from_node(ExprNode::Unary(
        rupa::core::ir::UnOp::IsNull,
        Box::new(Item::META.path("note").json().into_node()),
    ));
    assert_eq!(ids(e, note_json).await, Vec::<i64>::new());
}

pub async fn case_nullable_json<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    assert_eq!(
        ids(e, Item::EXTRA.is_null()).await,
        [1, 3, 4],
        "None is SQL NULL, not JSON null"
    );
    assert_eq!(
        ids(e, Item::EXTRA.path("score").cast::<i64>().eq(1)).await,
        [2]
    );
    update::<Item>()
        .set(Item::EXTRA, &None)
        .filter(Item::ID.eq(2))
        .affected()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(ids(e, Item::EXTRA.is_not_null()).await, Vec::<i64>::new());
}

pub async fn case_timestamps<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let cutoff = items()[1].created_at;
    assert_eq!(ids(e, Item::CREATED_AT.gt(cutoff)).await, [3, 4]);
    let latest = select::<Item>()
        .order_by(Item::CREATED_AT.desc())
        .limit(1)
        .one()
        .execute(e)
        .await
        .unwrap();
    assert_eq!(latest.created_at, items()[3].created_at);
}
