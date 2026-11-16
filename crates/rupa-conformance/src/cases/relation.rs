//! Updates and deletes through a `table!` marker, which has no row type.

use super::*;
use crate::fixture::{ItemRows, Stock, StockRow};

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

pub async fn case_backfill_reads_rows_and_writes_them_back<E: AsyncExecutor>(e: &E) {
    seed(e).await;

    let rows: Vec<StockRow> = select::<Stock>()
        .order_by(Stock::ID.asc())
        .all()
        .execute(e)
        .await
        .expect("read rows through the table");

    assert_eq!(rows.len(), items().len());

    for row in rows.iter().filter(|row| row.id <= 2) {
        let moved = StockRow {
            name: row.name.to_uppercase(),
            qty: row.qty * 2,
            ..row.clone()
        };

        let n = update::<Stock>()
            .one(&moved)
            .affected()
            .execute(e)
            .await
            .expect("write the row back");

        assert_eq!(n, 1);
    }

    let after = select::<Stock>()
        .order_by(Stock::ID.asc())
        .all()
        .execute(e)
        .await
        .expect("read again");

    for (before, now) in rows.iter().zip(&after) {
        if before.id <= 2 {
            assert_eq!(now.name, before.name.to_uppercase());
            assert_eq!(now.qty, before.qty * 2);
        } else {
            assert_eq!(now, before);
        }
    }

    assert_eq!(
        delete::<Stock>()
            .one(&after[0])
            .affected()
            .execute(e)
            .await
            .expect("delete a row"),
        1
    );
    assert_eq!(
        get::<Stock>(&after[0].id).execute(e).await.expect("get"),
        None
    );
}
