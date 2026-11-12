//! Dynamic dispatch (the default), unboxed futures (the opt-in), and
//! hand-written implementors.

use std::sync::Arc;

use rupa::{Database, DynError, DynRepository};

use super::*;

#[repository]
pub trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, DynError>;

    #[query(filter = active == $active, order_by = id)]
    fn active(&self, active: bool) -> Result<Vec<User>, DynError>;

    #[query(entity = User, filter = email == $email)]
    async fn email_taken(&self, email: &str) -> Result<bool, DynError>;

    /// Helper methods with a body are left alone.
    fn describe(&self) -> &'static str {
        "users"
    }
}

#[test]
fn dyn_repository_behind_arc() {
    let repo: Arc<dyn UserRepository> = Arc::new(UserRepositoryRepo::new(async_user_db()));
    let ada = pollster::block_on(repo.by_email("ada@x")).expect("by_email runs");
    let active = repo.active(true).expect("active runs");

    assert_eq!(ada.map(|u| u.id), Some(1));
    assert_eq!(active.iter().map(|u| u.id).collect::<Vec<_>>(), [1, 3]);
    assert!(pollster::block_on(repo.email_taken("cy@x")).expect("email_taken runs"));
    assert_eq!(repo.describe(), "users");
}

#[test]
fn dyn_repository_is_built_over_a_connection_without_naming_its_types() {
    let repo: Arc<dyn UserRepository> =
        <dyn UserRepository as DynRepository<_>>::over(Arc::new(async_user_db()));

    let ada = pollster::block_on(repo.by_email("ada@x")).expect("by_email runs");

    assert_eq!(ada.map(|u| u.id), Some(1));
}

#[test]
fn dyn_repository_errors_keep_their_kind() {
    #[repository]
    pub trait Strict: Send + Sync {
        #[query(filter = active == $active)]
        async fn exactly_one(&self, active: bool) -> Result<User, DynError>;
    }

    let repo: Arc<dyn Strict> = Arc::new(StrictRepo::new(Database::single(async_user_db())));
    let err = pollster::block_on(repo.exactly_one(true)).expect_err("two users are active");

    use rupa::ExecError;
    assert_eq!(err.result_error(), Some(&rupa::ResultError::TooManyRows));
    assert!(err.downcast_ref::<rupa::ResultError>().is_some());
}

#[repository(static_dispatch)]
pub trait FastUsers {
    #[query(filter = active == $active, order_by = id desc, limit = 1)]
    async fn newest_active(&self, active: bool) -> Result<Option<User>, AppError>;
}

#[test]
fn static_dispatch_repository_uses_unboxed_send_futures() {
    fn assert_send<T: Send>(_: &T) {}

    let db = async_user_db();
    let repo = FastUsersRepo::new(&db);
    let newest = pollster::block_on(repo.newest_active(true)).expect("newest_active runs");
    let future = repo.newest_active(false);

    assert_eq!(newest.map(|u| u.id), Some(3));
    assert_send(&future);
}

struct FakeUsers(Vec<User>);

impl UserRepository for FakeUsers {
    fn by_email(
        &self,
        email: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<User>, DynError>> + Send + '_>> {
        let found = self.0.iter().find(|u| u.email == email).cloned();

        Box::pin(async move { Ok(found) })
    }

    fn active(&self, active: bool) -> Result<Vec<User>, DynError> {
        Ok(self
            .0
            .iter()
            .filter(|u| u.active == active)
            .cloned()
            .collect())
    }

    fn email_taken(
        &self,
        email: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<bool, DynError>> + Send + '_>> {
        let taken = self.0.iter().any(|u| u.email == email);

        Box::pin(async move { Ok(taken) })
    }
}

#[test]
fn hand_written_mock_is_interchangeable() {
    let repos: Vec<Arc<dyn UserRepository>> = vec![
        Arc::new(FakeUsers(users())),
        Arc::new(UserRepositoryRepo::new(async_user_db())),
    ];

    for repo in repos {
        assert_eq!(repo.active(false).expect("active runs").len(), 1);
        assert!(pollster::block_on(repo.email_taken("ada@x")).expect("email_taken runs"));
    }
}
