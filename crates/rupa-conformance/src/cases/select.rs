//! Selects: filters, ordering, paging, JSON and timestamps.

use super::*;

pub async fn case_insert_and_select<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let all = e
        .run(select::<Item>().order_by(col!(Item::id).asc()).all())
        .await
        .unwrap();
    assert_eq!(all, items(), "rows must round-trip exactly");
}

pub async fn case_single_row_shapes<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let one = e
        .run(select::<Item>().filter(col!(Item::id).eq(2)).one())
        .await
        .unwrap();
    assert_eq!(one.name, "banana");
    let none = e
        .run(select::<Item>().filter(col!(Item::id).eq(99)).optional())
        .await
        .unwrap();
    assert_eq!(none, None);
    let some = e
        .run(select::<Item>().filter(col!(Item::id).eq(3)).optional())
        .await
        .unwrap();
    assert_eq!(some.map(|i| i.id), Some(3));

    let r = e
        .run(select::<Item>().filter(col!(Item::id).eq(99)).one())
        .await;
    assert_eq!(result_error(r), ResultError::NotFound);
    let r = e
        .run(select::<Item>().filter(col!(Item::active).eq(true)).one())
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);
    let r = e
        .run(
            select::<Item>()
                .filter(col!(Item::active).eq(true))
                .optional(),
        )
        .await;
    assert_eq!(result_error(r), ResultError::TooManyRows);
}

pub async fn case_comparisons<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(ids(e, col!(Item::qty).eq(7)).await, [3]);
    assert_eq!(ids(e, col!(Item::qty).ne(7)).await, [1, 2, 4]);
    assert_eq!(ids(e, col!(Item::qty).lt(7)).await, [1, 4]);
    assert_eq!(ids(e, col!(Item::qty).le(7)).await, [1, 3, 4]);
    assert_eq!(ids(e, col!(Item::qty).gt(3)).await, [2, 3]);
    assert_eq!(ids(e, col!(Item::qty).ge(3)).await, [1, 2, 3]);
    assert_eq!(ids(e, col!(Item::name).gt("banana")).await, [3, 4]);
    assert_eq!(
        ids(e, col!(Item::active).eq(false).or(col!(Item::qty).eq(0))).await,
        [2, 4]
    );
}

/// NULL = 'red' is NULL, and NOT NULL is still NULL: neither keeps rows 2 and 4.
///
/// NULL OR TRUE is TRUE; NULL AND FALSE is FALSE.
pub async fn case_three_valued_logic<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(ids(e, col!(Item::label).eq("red")).await, [1]);
    assert_eq!(ids(e, !col!(Item::label).eq("red")).await, [3]);
    assert_eq!(ids(e, col!(Item::label).is_null()).await, [2, 4]);
    assert_eq!(ids(e, col!(Item::label).is_not_null()).await, [1, 3]);
    assert_eq!(
        ids(e, col!(Item::label).eq("x").or(col!(Item::qty).eq(12))).await,
        [2]
    );
    assert_eq!(
        ids(e, !(col!(Item::label).eq("x").and(col!(Item::qty).eq(-5)))).await,
        [1, 2, 3, 4]
    );
}

/// x NOT IN (1, NULL) is never TRUE.
pub async fn case_in_lists<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(ids(e, col!(Item::id).in_([1i64, 3, 99])).await, [1, 3]);
    assert_eq!(ids(e, col!(Item::id).not_in([1i64, 3])).await, [2, 4]);
    assert_eq!(
        ids(e, col!(Item::id).in_(Vec::<i64>::new())).await,
        Vec::<i64>::new()
    );
    assert_eq!(
        ids(e, col!(Item::id).not_in(Vec::<i64>::new())).await,
        [1, 2, 3, 4]
    );
    let null =
        Expr::<Option<i64>>::from_node(ExprNode::Param(Value::Null(rupa::core::SqlType::I64)));
    assert_eq!(
        ids(
            e,
            col!(Item::id).not_in([Expr::from_node(ExprNode::Param(Value::I64(1))), null])
        )
        .await,
        Vec::<i64>::new()
    );
}

pub async fn case_like_patterns<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(ids(e, col!(Item::name).like("%an%")).await, [2]);
    assert_eq!(ids(e, col!(Item::name).like("_pple")).await, [1]);
    assert_eq!(ids(e, col!(Item::label).like("dark\\_%")).await, [3]);
    assert_eq!(ids(e, col!(Item::label).like("%")).await, [1, 3]);
}

pub async fn case_order_with_nulls<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let asc = e
        .run(
            select::<Item>()
                .order_by(col!(Item::label).asc())
                .order_by(col!(Item::id).asc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        asc.iter().map(|i| i.id).collect::<Vec<_>>(),
        [3, 1, 2, 4],
        "NULLS LAST ascending"
    );
    let desc = e
        .run(
            select::<Item>()
                .order_by(col!(Item::label).desc())
                .order_by(col!(Item::id).desc())
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(
        desc.iter().map(|i| i.id).collect::<Vec<_>>(),
        [4, 2, 1, 3],
        "NULLS FIRST descending"
    );
}

pub async fn case_limit_offset<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let page = e
        .run(
            select::<Item>()
                .order_by(col!(Item::id).asc())
                .limit(2)
                .offset(1)
                .all(),
        )
        .await
        .unwrap();
    assert_eq!(page.iter().map(|i| i.id).collect::<Vec<_>>(), [2, 3]);
    let past_end = e
        .run(
            select::<Item>()
                .order_by(col!(Item::id).asc())
                .offset(10)
                .all(),
        )
        .await
        .unwrap();
    assert!(past_end.is_empty());
    let zero = e.run(select::<Item>().limit(0).all()).await.unwrap();
    assert!(zero.is_empty());
}

pub async fn case_exists<E: AsyncExecutor>(e: &mut E) {
    assert!(
        !e.run(select::<Item>().exists()).await.unwrap(),
        "empty table"
    );
    seed(e).await;
    assert!(
        e.run(
            select::<Item>()
                .filter(col!(Item::name).eq("cherry"))
                .exists()
        )
        .await
        .unwrap()
    );
    assert!(
        !e.run(select::<Item>().filter(col!(Item::name).eq("fig")).exists())
            .await
            .unwrap()
    );
}

/// A missing key and a JSON null are both SQL NULL under ->>.
///
/// ...but under ->, a JSON null is a value, not SQL NULL.
pub async fn case_json_paths<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(
        ids(e, col!(Item::meta).path("nested").key("k").text().eq("w")).await,
        [3]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("tags").index(0).text().eq("a")).await,
        [1]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("flag").cast::<bool>().eq(true)).await,
        [1, 3]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("score").cast::<i64>().gt(6)).await,
        [2, 3]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("missing").text().is_null()).await,
        [1, 2, 3, 4]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("note").text().is_null()).await,
        [1, 2, 3, 4]
    );
    assert_eq!(
        ids(e, col!(Item::meta).path("nested").key("k").text().is_null()).await,
        [1, 4]
    );
    let note_json = Expr::<bool>::from_node(ExprNode::Unary(
        rupa::core::ir::UnOp::IsNull,
        Box::new(col!(Item::meta).path("note").json().into_node()),
    ));
    assert_eq!(ids(e, note_json).await, Vec::<i64>::new());
}

pub async fn case_nullable_json<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    assert_eq!(
        ids(e, col!(Item::extra).is_null()).await,
        [1, 3, 4],
        "None is SQL NULL, not JSON null"
    );
    assert_eq!(
        ids(e, col!(Item::extra).path("score").cast::<i64>().eq(1)).await,
        [2]
    );
    e.run(
        update::<Item>()
            .set(col!(Item::extra), &None)
            .filter(col!(Item::id).eq(2))
            .affected(),
    )
    .await
    .unwrap();
    assert_eq!(
        ids(e, col!(Item::extra).is_not_null()).await,
        Vec::<i64>::new()
    );
}

pub async fn case_timestamps<E: AsyncExecutor>(e: &mut E) {
    seed(e).await;
    let cutoff = items()[1].created_at;
    assert_eq!(ids(e, col!(Item::created_at).gt(cutoff)).await, [3, 4]);
    let latest = e
        .run(
            select::<Item>()
                .order_by(col!(Item::created_at).desc())
                .limit(1)
                .one(),
        )
        .await
        .unwrap();
    assert_eq!(latest.created_at, items()[3].created_at);
}
