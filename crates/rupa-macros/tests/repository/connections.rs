//! One repository type over every connection: a database, a transaction it
//! opens or joins, and any wrapper of either.

use std::rc::Rc;
use std::sync::Arc;

use rupa::{Connection, Immediate, TxOptions};

use super::*;

#[repository]
pub trait Users {
    #[query(insert)]
    fn add(&self, user: &User) -> Result<u64, AppError>;

    #[query(filter = id == $id)]
    fn by_id(&self, id: i64) -> Result<Option<User>, AppError>;

    #[query(filter = active == $active, order_by = id)]
    fn active(&self, active: bool) -> Result<Vec<User>, AppError>;
}

#[repository]
pub trait AsyncUsers: Send + Sync {
    #[query(insert)]
    async fn add(&self, user: &User) -> Result<u64, AppError>;

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

/// The ids of the active users `users` sees.
fn active_ids<C: Connection<Mode = Immediate>>(users: &UsersRepo<C>) -> Vec<i64> {
    let active = users.active(true).expect("active runs");

    active.iter().map(|u| u.id).collect()
}

#[test]
fn a_repository_opens_and_ends_its_own_transactions() {
    let db = user_db();
    let users = UsersRepo::new(&db);

    let tx = users.open_tx(None).expect("transaction opens");

    assert_eq!(tx.add(&dan()).expect("add runs"), 1);
    assert_eq!(active_ids(&tx), [1, 3, 4], "sees its own write");
    tx.rollback().expect("rollback runs");
    assert_eq!(active_ids(&users), [1, 3], "rolled back");

    let tx = users.open_tx(None).expect("transaction opens");

    tx.add(&dan()).expect("add runs");
    tx.commit().expect("commit runs");
    assert_eq!(active_ids(&users), [1, 3, 4], "committed");
}

#[test]
fn a_dropped_transaction_rolls_back() {
    let db = user_db();
    let users = UsersRepo::new(&db);

    users
        .open_tx(None)
        .expect("transaction opens")
        .add(&dan())
        .expect("add runs");

    assert_eq!(active_ids(&users), [1, 3]);
}

#[test]
fn transaction_options_are_passed_through() {
    let db = user_db();
    let users = UsersRepo::new(&db);
    let tx = users
        .open_tx(Some(TxOptions::default().read_only()))
        .expect("read-only transaction opens");

    assert!(
        tx.add(&dan()).is_err(),
        "a read-only transaction refuses writes"
    );
}

#[test]
fn a_repository_joins_an_existing_transaction() {
    let db = user_db();
    let users = UsersRepo::new(&db);
    let tx = db.begin().expect("transaction opens");

    users.tx(&tx).add(&dan()).expect("add runs");
    assert_eq!(active_ids(&users.tx(&tx)), [1, 3, 4], "joined the tx");
    tx.rollback().expect("rollback runs");
    assert_eq!(active_ids(&users), [1, 3], "rolled back with it");
}

#[test]
fn a_transaction_is_passed_from_one_repository_to_another() {
    #[repository]
    pub trait Audit {
        #[query(filter = active == true)]
        fn active(&self) -> Result<Vec<User>, AppError>;
    }

    let db = user_db();
    let users = UsersRepo::new(&db);
    let audit = AuditRepo::new(&db);
    let tx = users.open_tx(None).expect("transaction opens");

    tx.add(&dan()).expect("add runs");

    assert_eq!(
        audit
            .tx(tx.connection())
            .active()
            .expect("active runs")
            .len(),
        3
    );
    tx.commit().expect("commit runs");
    assert_eq!(audit.active().expect("active runs").len(), 3);
}

#[test]
fn a_transaction_repository_opens_savepoints() {
    let db = user_db();
    let users = UsersRepo::new(&db);
    let tx = users.open_tx(None).expect("transaction opens");
    let savepoint = tx.open_tx(None).expect("savepoint opens");

    savepoint.add(&dan()).expect("add runs");
    savepoint.rollback().expect("savepoint rolls back");
    assert_eq!(active_ids(&tx), [1, 3]);
    tx.commit().expect("commit runs");
}

#[test]
fn repositories_run_on_any_wrapper_of_a_connection() {
    let db = user_db();
    let shared = Arc::new(user_db());
    let boxed = UsersRepo::new(Box::new(user_db()));
    let counted = UsersRepo::new(Rc::new(user_db()));
    let owned = UsersRepo::new(user_db());

    assert_eq!(active_ids(&UsersRepo::new(&db)), [1, 3]);
    assert_eq!(active_ids(&UsersRepo::new(Arc::clone(&shared))), [1, 3]);
    assert_eq!(active_ids(&boxed), [1, 3]);
    assert_eq!(active_ids(&counted), [1, 3]);
    assert_eq!(active_ids(&owned), [1, 3]);
    assert!(owned.into_connection().rows(User::TABLE).is_some());
}

#[test]
fn async_repositories_branch_into_transactions_on_sync_connections() {
    let db = user_db();
    let users = AsyncUsersRepo::new(&db);

    pollster::block_on(async {
        let tx = users.open_tx(None).await.expect("transaction opens");

        tx.add(&dan()).await.expect("add runs");
        assert!(tx.by_id(4).await.expect("by_id runs").is_some());
        tx.rollback().await.expect("rollback runs");
        assert!(users.by_id(4).await.expect("by_id runs").is_none());
    });
}

#[test]
fn async_repositories_branch_into_async_transactions() {
    let db = async_user_db();
    let users = AsyncUsersRepo::new(&db);

    pollster::block_on(async {
        let tx = users.open_tx(None).await.expect("transaction opens");

        tx.add(&dan()).await.expect("add runs");
        assert!(tx.by_id(4).await.expect("by_id runs").is_some());
        tx.commit().await.expect("commit runs");
        assert!(users.by_id(4).await.expect("by_id runs").is_some());
    });
}

#[test]
fn a_sync_repository_is_a_trait_object_too() {
    let db = user_db();
    let repo = UsersRepo::new(&db);
    let users: &dyn Users = &repo;

    assert!(users.by_id(9).expect("by_id runs").is_none());
    assert_eq!(
        users.by_id(2).expect("by_id runs").map(|u| u.email),
        Some("bob@x".to_string())
    );
}
