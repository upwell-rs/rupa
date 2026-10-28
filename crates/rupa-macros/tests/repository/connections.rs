//! One repository type over every connection: a database, a transaction
//! shared by several repositories, and any wrapper of either.

use std::rc::Rc;
use std::sync::Arc;

use rupa::Connection;

use super::*;

#[repository]
pub trait UserReads {
    #[query(filter = id == $id)]
    fn by_id(&self, id: i64) -> Result<Option<User>, AppError>;

    #[query(filter = active == $active, order_by = id)]
    fn active(&self, active: bool) -> Result<Vec<User>, AppError>;
}

#[repository]
pub trait UserWrites {
    #[query(insert)]
    fn add(&self, user: &User) -> Result<u64, AppError>;
}

#[repository]
pub trait AsyncUserReads: Send + Sync {
    #[query(filter = id == $id)]
    async fn by_id(&self, id: i64) -> Result<Option<User>, AppError>;
}

fn dan() -> User {
    User {
        id: 4,
        email: "dan@x".into(),
        active: true,
    }
}

/// The ids of the active users `reads` sees.
fn active_ids<C: Connection<Mode = rupa::Immediate>>(reads: &UserReadsRepo<C>) -> Vec<i64> {
    let active = reads.active(true).expect("active runs");

    active.iter().map(|u| u.id).collect()
}

#[test]
fn repositories_share_a_transaction_by_reference() {
    let db = user_db();
    let tx = db.begin().expect("transaction opens");
    let writes = UserWritesRepo::new(&tx);
    let reads = UserReadsRepo::new(&tx);

    assert_eq!(writes.add(&dan()).expect("add runs"), 1);
    assert_eq!(active_ids(&reads), [1, 3, 4], "sees the shared tx's write");
    tx.rollback().expect("rollback runs");
    assert_eq!(active_ids(&UserReadsRepo::new(&db)), [1, 3], "rolled back");

    let tx = db.begin().expect("transaction opens");

    UserWritesRepo::new(&tx).add(&dan()).expect("add runs");
    tx.commit().expect("commit runs");
    assert_eq!(active_ids(&UserReadsRepo::new(&db)), [1, 3, 4], "committed");
}

#[test]
fn a_savepoint_is_a_connection_too() {
    let db = user_db();
    let tx = db.begin().expect("transaction opens");
    let savepoint = tx.begin().expect("savepoint opens");

    UserWritesRepo::new(&savepoint)
        .add(&dan())
        .expect("add runs");
    savepoint.rollback().expect("savepoint rolls back");
    assert_eq!(active_ids(&UserReadsRepo::new(&tx)), [1, 3]);
}

#[test]
fn repositories_run_on_any_wrapper_of_a_connection() {
    let db = user_db();
    let shared = Arc::new(user_db());
    let boxed = UserReadsRepo::new(Box::new(user_db()));
    let counted = UserReadsRepo::new(Rc::new(user_db()));
    let owned = UserReadsRepo::new(user_db());

    assert_eq!(active_ids(&UserReadsRepo::new(&db)), [1, 3]);
    assert_eq!(active_ids(&UserReadsRepo::new(Arc::clone(&shared))), [1, 3]);
    assert_eq!(active_ids(&boxed), [1, 3]);
    assert_eq!(active_ids(&counted), [1, 3]);
    assert_eq!(active_ids(&owned), [1, 3]);
    assert!(owned.into_connection().rows(User::TABLE).is_some());
}

#[test]
fn async_repositories_run_on_sync_connections() {
    let db = user_db();
    let tx = db.begin().expect("transaction opens");
    let on_db = AsyncUserReadsRepo::new(&db);
    let on_tx = AsyncUserReadsRepo::new(&tx);

    UserWritesRepo::new(&tx).add(&dan()).expect("add runs");

    assert!(
        pollster::block_on(on_tx.by_id(4))
            .expect("by_id runs")
            .is_some()
    );
    assert!(
        pollster::block_on(on_db.by_id(1))
            .expect("by_id runs")
            .is_some()
    );
}

#[test]
fn async_repositories_share_an_async_transaction() {
    let db = async_user_db();
    let tx = pollster::block_on(db.begin()).expect("transaction opens");
    let reads = AsyncUserReadsRepo::new(&tx);

    pollster::block_on(
        insert::<User>()
            .value(&dan())
            .affected()
            .execute(reads.connection()),
    )
    .expect("insert runs");

    assert!(
        pollster::block_on(reads.by_id(4))
            .expect("by_id runs")
            .is_some()
    );
    pollster::block_on(tx.rollback()).expect("rollback runs");
    assert!(
        pollster::block_on(AsyncUserReadsRepo::new(&db).by_id(4))
            .expect("by_id runs")
            .is_none()
    );
}

#[test]
fn a_sync_repository_is_a_trait_object_too() {
    let db = user_db();
    let repo = UserReadsRepo::new(&db);
    let reads: &dyn UserReads = &repo;

    assert!(reads.by_id(9).expect("by_id runs").is_none());
    assert_eq!(
        reads.by_id(2).expect("by_id runs").map(|u| u.email),
        Some("bob@x".to_string())
    );
}
