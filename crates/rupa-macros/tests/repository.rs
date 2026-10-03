//! `#[repository]` end to end on the memory backend.

use std::sync::Arc;

use rupa::core::exec::{AsyncExecutor, Executor};
use rupa::core::tx::Transactional;
use rupa::prelude::*;
use rupa::{DynError, Shared};
use rupa_driver_memory::{AsyncMemoryDb, MemoryDb, MemoryError};

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "users")]
pub struct User {
    #[id]
    id: i64,
    email: String,
    active: bool,
}

fn users() -> Vec<User> {
    [(1, "ada@x", true), (2, "bob@x", false), (3, "cy@x", true)]
        .map(|(id, email, active)| User {
            id,
            email: email.into(),
            active,
        })
        .to_vec()
}

// ---------------------------------------------------------------------------
// The default: dyn-compatible, `&self`, async (one method blocks in place).
// ---------------------------------------------------------------------------

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

fn async_db() -> AsyncMemoryDb {
    let mut db = MemoryDb::new();
    db.register::<User>();
    db.run(insert::<User>().values(&users()).affected())
        .unwrap();
    db.into_async()
}

#[test]
fn dyn_repository_behind_arc() {
    let repo: Arc<dyn UserRepository> = Arc::new(Repo::shared_async(async_db()));
    let ada = pollster::block_on(repo.by_email("ada@x")).unwrap();
    assert_eq!(ada.map(|u| u.id), Some(1));
    assert_eq!(
        repo.active(true)
            .unwrap()
            .iter()
            .map(|u| u.id)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert!(pollster::block_on(repo.email_taken("cy@x")).unwrap());
    assert_eq!(repo.describe(), "users");
}

#[test]
fn dyn_repository_errors_keep_their_kind() {
    #[repository]
    pub trait Strict: Send + Sync {
        #[query(filter = active == $active)]
        async fn exactly_one(&self, active: bool) -> Result<User, DynError>;
    }
    let repo: Arc<dyn Strict> = Arc::new(Repo::shared_async(async_db()));
    let err = pollster::block_on(repo.exactly_one(true)).unwrap_err();
    use rupa::core::exec::ExecError;
    assert_eq!(
        err.result_error(),
        Some(&rupa::core::ResultError::TooManyRows)
    );
}

// ---------------------------------------------------------------------------
// Sync, `&self`, through a mutex-shared executor; own error type.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum AppError {
    Db(MemoryError),
}

impl From<MemoryError> for AppError {
    fn from(e: MemoryError) -> Self {
        AppError::Db(e)
    }
}

#[repository]
pub trait SyncUsers {
    #[query(filter = id == $id)]
    fn by_id(&self, id: i64) -> Result<Option<User>, AppError>;
}

#[test]
fn sync_repository_over_shared_executor() {
    let mut db = MemoryDb::new();
    db.register::<User>();
    db.run(insert::<User>().values(&users()).affected())
        .unwrap();
    let repo = Repo::new(Shared::new(db));
    assert_eq!(
        repo.by_id(2).unwrap().map(|u| u.email),
        Some("bob@x".to_string())
    );
    let shared: &dyn SyncUsers = &repo;
    assert!(shared.by_id(9).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// `&mut self`: one executor, e.g. inside a transaction.
// ---------------------------------------------------------------------------

#[repository]
pub trait TxUsers {
    #[query(filter = active == $active)]
    fn active(&mut self, active: bool) -> Result<Vec<User>, MemoryError>;
}

#[test]
fn mut_repository_inside_a_transaction() {
    let mut db = MemoryDb::new();
    db.register::<User>();
    let mut tx = db.begin().unwrap();
    tx.run(insert::<User>().values(&users()).affected())
        .unwrap();
    {
        let mut repo = Repo::new(&mut tx);
        assert_eq!(
            repo.active(true).unwrap().len(),
            2,
            "sees the tx's own writes"
        );
    }
    drop(tx);
    assert_eq!(
        Repo::new(&mut db).active(true).unwrap().len(),
        0,
        "rolled back"
    );
}

// ---------------------------------------------------------------------------
// static_dispatch: unboxed futures.
// ---------------------------------------------------------------------------

#[repository(static_dispatch)]
pub trait FastUsers {
    #[query(filter = active == $active, order_by = id desc, limit = 1)]
    async fn newest_active(&mut self, active: bool) -> Result<Option<User>, MemoryError>;
}

#[test]
fn static_dispatch_repository() {
    let mut db = async_db();
    let mut repo = Repo::new(&mut db);
    let newest = pollster::block_on(repo.newest_active(true)).unwrap();
    assert_eq!(newest.map(|u| u.id), Some(3));
    // The future is Send, so it can be spawned.
    fn assert_send<T: Send>(_: &T) {}
    let fut = repo.newest_active(false);
    assert_send(&fut);
    drop(fut);
    let _ = AsyncExecutor::dialect(&db);
}

// ---------------------------------------------------------------------------
// Hand-written implementors: the trait is an ordinary trait.
// ---------------------------------------------------------------------------

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
        Arc::new(Repo::shared_async(async_db())),
    ];
    for repo in repos {
        assert_eq!(repo.active(false).unwrap().len(), 1);
        assert!(pollster::block_on(repo.email_taken("ada@x")).unwrap());
    }
}

// ---------------------------------------------------------------------------
// CRUD methods, declared with attributes.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "notes")]
pub struct Note {
    #[id(generated)]
    id: i64,
    title: String,
    body: Option<String>,
}

#[derive(Insertable)]
#[insertable(entity = Note)]
pub struct NewNote {
    title: String,
}

#[derive(Updatable)]
#[updatable(entity = Note)]
pub struct NotePatch {
    #[id]
    id: i64,
    body: Option<Option<String>>,
}

#[repository]
pub trait NoteRepository: Send + Sync {
    #[query(get)]
    async fn get(&self, id: i64) -> Result<Option<Note>, DynError>;

    #[query(insert, returning)]
    async fn create(&self, note: &NewNote) -> Result<Note, DynError>;

    #[query(insert, returning)]
    async fn create_many(&self, notes: &[NewNote]) -> Result<Vec<Note>, DynError>;

    #[query(insert)]
    fn import(&self, note: &Note) -> Result<u64, DynError>;

    #[query(update)]
    async fn patch(&self, patch: &NotePatch) -> Result<bool, DynError>;

    #[query(update)]
    fn save(&self, note: &Note) -> Result<u64, DynError>;

    #[query(delete)]
    async fn remove(&self, note: &Note) -> Result<bool, DynError>;

    #[query(delete, entity = Note)]
    async fn remove_id(&self, id: i64) -> Result<u64, DynError>;

    #[query(delete, entity = Note, filter = body is_null)]
    fn purge_empty(&self) -> Result<u64, DynError>;
}

fn note_db() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<Note>();
    db
}

fn exercise_notes(repo: &dyn NoteRepository) {
    fn block<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }
    let a = block(repo.create(&NewNote { title: "a".into() })).unwrap();
    assert_eq!(
        a,
        Note {
            id: 1,
            title: "a".into(),
            body: None
        }
    );
    let more =
        block(repo.create_many(&[NewNote { title: "b".into() }, NewNote { title: "c".into() }]))
            .unwrap();
    assert_eq!(more.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(
        repo.import(&Note {
            id: 0,
            title: "d".into(),
            body: Some("x".into())
        })
        .unwrap(),
        1
    );

    assert!(
        block(repo.patch(&NotePatch {
            id: 1,
            body: Some(Some("hello".into()))
        }))
        .unwrap()
    );
    let mut a = block(repo.get(1)).unwrap().unwrap();
    assert_eq!(a.body.as_deref(), Some("hello"));
    a.title = "a2".into();
    assert_eq!(repo.save(&a).unwrap(), 1);
    assert_eq!(block(repo.get(1)).unwrap().unwrap().title, "a2");

    assert!(block(repo.remove(&a)).unwrap());
    assert_eq!(block(repo.remove_id(2)).unwrap(), 1);
    assert_eq!(block(repo.remove_id(2)).unwrap(), 0);
    assert_eq!(repo.purge_empty().unwrap(), 1, "only `c` has no body");
    assert_eq!(
        block(repo.get(4)).unwrap().map(|n| n.title),
        Some("d".to_string())
    );
}

#[test]
fn crud_repository_over_an_async_executor() {
    let repo: Arc<dyn NoteRepository> = Arc::new(Repo::shared_async(note_db().into_async()));
    exercise_notes(&*repo);
}

/// A sync-only application (think SQLite) declaring `async` methods: the
/// plain sync executor serves them, and no async runtime is involved.
#[test]
fn async_repository_over_a_sync_executor() {
    let repo: Arc<dyn NoteRepository> = Arc::new(Repo::shared(note_db()));
    exercise_notes(&*repo);
}

#[repository]
pub trait OneConnection {
    #[query(get)]
    async fn get(&mut self, id: i64) -> Result<Option<Note>, MemoryError>;
}

/// `&mut self` over one sync executor (or transaction), made async with `Blocking`.
#[test]
fn blocking_wraps_one_sync_executor() {
    use rupa::core::exec::Blocking;
    let mut db = note_db();
    db.run(
        insert::<Note>()
            .value(&NewNote { title: "a".into() })
            .affected(),
    )
    .unwrap();
    let mut tx = db.begin().unwrap();
    let mut repo = Repo::new(Blocking(&mut tx));
    assert!(pollster::block_on(repo.get(1)).unwrap().is_some());
}
