//! DSL functions: lowering and evaluation agree.

use super::*;

pub async fn case_dsl_lowering_and_eval_agree<E: AsyncExecutor>(e: &E) {
    seed(e).await;
    let ci = |s: &str| ci_eq(Item::NAME, s);
    assert_eq!(ids(e, ci("CHERRY")).await, [3]);
    assert_eq!(ids(e, !ci("Cherry")).await, [1, 2, 4]);
}

pub async fn case_dsl_std_text<E: AsyncExecutor>(e: &E) {
    use rupa::dsl::{ilike, lower, upper};
    seed(e).await;
    assert_eq!(ids(e, ilike(Item::NAME, "%AN%")).await, [2]);
    assert_eq!(
        ids(e, ilike(Item::LABEL, "DARK%")).await,
        [3],
        "NULL labels do not match"
    );
    assert_eq!(ids(e, lower(Item::NAME).eq("cherry")).await, [3]);
    assert_eq!(ids(e, upper(Item::NAME).eq("DATE")).await, [4]);
}

pub async fn case_dsl_std_json<E: AsyncExecutor>(e: &E) {
    use rupa::dsl::{json_contains, json_has_key};
    seed(e).await;
    assert_eq!(
        ids(e, json_has_key(Item::EXTRA, "score")).await,
        [2],
        "SQL NULL `extra` is not a match"
    );
    assert_eq!(
        ids(e, !json_has_key(Item::META, "missing")).await,
        [1, 2, 3, 4]
    );
    if !e.dialect().supports(Capability::JsonContainment) {
        return;
    }

    assert_eq!(
        ids(
            e,
            json_contains(Item::META, serde_json::json!({"tags": ["a"]}))
        )
        .await,
        [1]
    );
    assert_eq!(
        ids(
            e,
            json_contains(Item::META, serde_json::json!({"flag": true}))
        )
        .await,
        [1, 3]
    );
    assert_eq!(
        ids(
            e,
            json_contains(Item::META, serde_json::json!({"nested": {"k": "w"}}))
        )
        .await,
        [3]
    );
    assert_eq!(
        ids(e, json_contains(Item::META, serde_json::json!({}))).await,
        [1, 2, 3, 4]
    );
}
