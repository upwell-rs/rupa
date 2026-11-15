//! Updates and deletes through a `table!` marker, which has no row type.

use super::*;
use crate::fixture::ItemRows;

pub async fn case_update_and_delete_through_a_table_marker<E: AsyncExecutor>(e: &E) {
    seed(e).await;

    let backfilled = update::<ItemRows>()
        .set(ItemRows::QTY, &100)
        .set(ItemRows::LABEL, &Some("backfilled".to_owned()))
        .set(ItemRows::NAME, &"renamed".to_owned())
        .filter(ItemRows::ACTIVE.eq(true))
        .affected()
        .execute(e)
        .await
        .expect("update through the marker");

    assert_eq!(backfilled, 3);

    let rows = select::<Item>()
        .order_by(Item::ID.asc())
        .all()
        .execute(e)
        .await
        .expect("select");

    for item in &rows {
        if item.active {
            assert_eq!((item.qty, item.name.as_str()), (100, "renamed"));
            assert_eq!(item.label.as_deref(), Some("backfilled"));
        } else {
            assert_eq!(item, &items()[1]);
        }
    }

    let deleted = delete::<ItemRows>()
        .filter(ItemRows::QTY.eq(100))
        .affected()
        .execute(e)
        .await
        .expect("delete through the marker");

    assert_eq!(deleted, 3);
    assert_eq!(ids(e, Item::ID.gt(0)).await, [2]);
}
