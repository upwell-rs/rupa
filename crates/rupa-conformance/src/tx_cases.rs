//! The transaction cases every driver must pass.

use super::*;

fn note(title: &str) -> NewNote {
    NewNote {
        title: title.into(),
        pinned: false,
    }
}

async fn titles<E: AsyncExecutor>(e: &mut E) -> Vec<String> {
    let notes = e
        .run(select::<Note>().order_by(col!(Note::id).asc()).all())
        .await
        .expect("select");
    notes.into_iter().map(|n| n.title).collect()
}

async fn add<E: AsyncExecutor>(e: &mut E, title: &str) {
    e.run(insert::<Note>().value(&note(title)).affected())
        .await
        .expect("insert");
}

/// An error type for closure transactions: a database error, or a
/// deliberate abort.
#[derive(Debug)]
enum Abort<X> {
    Db(#[allow(dead_code)] X),
    Explicit,
}

impl<X> From<X> for Abort<X> {
    fn from(e: X) -> Self {
        Abort::Db(e)
    }
}

pub async fn tx_commit_persists<E: AsyncTransactional>(e: &mut E) {
    let mut tx = e.begin().await.unwrap();
    add(&mut tx, "a").await;
    tx.commit().await.unwrap();
    assert_eq!(titles(e).await, ["a"]);
}

pub async fn tx_rollback_discards<E: AsyncTransactional>(e: &mut E) {
    add(e, "kept").await;
    let mut tx = e.begin().await.unwrap();
    add(&mut tx, "gone").await;
    assert_eq!(
        titles(&mut tx).await,
        ["kept", "gone"],
        "a tx sees its own writes"
    );
    tx.rollback().await.unwrap();
    assert_eq!(titles(e).await, ["kept"]);
}

pub async fn tx_drop_rolls_back<E: AsyncTransactional>(e: &mut E) {
    {
        let mut tx = e.begin().await.unwrap();
        add(&mut tx, "gone").await;
    }
    assert!(
        titles(e).await.is_empty(),
        "a dropped tx must not leave its writes behind"
    );
}

pub async fn tx_is_an_executor<E: AsyncTransactional>(e: &mut E) {
    /// Generic over any async executor: works on a connection or a tx.
    async fn count<X: AsyncExecutor>(x: &mut X) -> usize {
        x.run(select::<Note>().all()).await.unwrap().len()
    }
    let mut tx = e.begin().await.unwrap();
    add(&mut tx, "a").await;
    assert_eq!(count(&mut tx).await, 1);
    tx.commit().await.unwrap();
    assert_eq!(count(e).await, 1);
}

pub async fn tx_nested_rollback_keeps_outer<E: AsyncTransactional>(e: &mut E) {
    let mut outer = e.begin().await.unwrap();
    add(&mut outer, "outer").await;
    let mut inner = outer.begin().await.unwrap();
    add(&mut inner, "inner").await;
    inner.rollback().await.unwrap();
    add(&mut outer, "after").await;
    outer.commit().await.unwrap();
    assert_eq!(titles(e).await, ["outer", "after"]);
}

pub async fn tx_nested_drop_keeps_outer<E: AsyncTransactional>(e: &mut E) {
    let mut outer = e.begin().await.unwrap();
    add(&mut outer, "outer").await;
    {
        let mut inner = outer.begin().await.unwrap();
        add(&mut inner, "inner").await;
    }
    assert_eq!(
        titles(&mut outer).await,
        ["outer"],
        "the dropped savepoint is rolled back"
    );
    outer.commit().await.unwrap();
    assert_eq!(titles(e).await, ["outer"]);
}

pub async fn tx_nested_commit_outer_rollback<E: AsyncTransactional>(e: &mut E) {
    let mut outer = e.begin().await.unwrap();
    let mut inner = outer.begin().await.unwrap();
    add(&mut inner, "inner").await;
    inner.commit().await.unwrap();
    assert_eq!(titles(&mut outer).await, ["inner"]);
    outer.rollback().await.unwrap();
    assert!(
        titles(e).await.is_empty(),
        "committing a savepoint does not commit the outer tx"
    );
}

pub async fn tx_closure_commits_or_rolls_back<E: AsyncTransactional>(e: &mut E) {
    let ok: Result<u64, Abort<E::Error>> = e
        .transaction(async |tx: &mut E::Tx<'_>| {
            Ok(tx
                .run(insert::<Note>().value(&note("ok")).affected())
                .await?)
        })
        .await;
    assert_eq!(ok.unwrap(), 1);

    let aborted: Result<(), Abort<E::Error>> = e
        .transaction(async |tx: &mut E::Tx<'_>| {
            tx.run(insert::<Note>().value(&note("aborted")).affected())
                .await?;
            Err(Abort::Explicit)
        })
        .await;
    assert!(matches!(aborted, Err(Abort::Explicit)));
    assert_eq!(titles(e).await, ["ok"]);
}

/// Requesting it must be an error, never silently ignored.
pub async fn tx_read_only_rejects_writes<E: AsyncTransactional>(e: &mut E) {
    add(e, "a").await;
    if !e.dialect().supports(Capability::ReadOnlyTransactions) {
        assert!(
            e.begin_with(TxOptions::default().read_only())
                .await
                .is_err()
        );
        assert_eq!(titles(e).await, ["a"]);
        return;
    }

    let mut tx = e
        .begin_with(TxOptions::default().read_only())
        .await
        .unwrap();
    assert_eq!(titles(&mut tx).await, ["a"], "reads work");
    let write = tx.run(insert::<Note>().value(&note("b")).affected()).await;
    assert!(
        write.is_err(),
        "writes in a read-only transaction must fail"
    );
    tx.rollback().await.unwrap();
    assert_eq!(titles(e).await, ["a"]);
}

pub async fn tx_options_on_nested_are_rejected<E: AsyncTransactional>(e: &mut E) {
    let mut tx = e.begin().await.unwrap();
    let rejected = tx
        .begin_with(TxOptions::default().read_only())
        .await
        .is_err();
    assert!(
        rejected,
        "options on a savepoint must be an error, not ignored"
    );
    tx.rollback().await.unwrap();
}

pub async fn tx_isolation_levels<E: AsyncTransactional>(e: &mut E) {
    for level in [
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ] {
        let mut tx = e
            .begin_with(TxOptions::default().isolation(level))
            .await
            .unwrap();
        add(&mut tx, "x").await;
        tx.commit().await.unwrap();
    }

    assert_eq!(titles(e).await.len(), 3);
}

pub async fn tx_sequential_after_drop<E: AsyncTransactional>(e: &mut E) {
    drop(e.begin().await.unwrap());
    let mut tx = e.begin().await.unwrap();
    add(&mut tx, "second").await;
    tx.commit().await.unwrap();
    let mut tx = e.begin().await.unwrap();
    let mut inner = tx.begin().await.unwrap();
    add(&mut inner, "nested").await;
    drop(inner);
    drop(tx);
    assert_eq!(titles(e).await, ["second"]);
}
