//! CRUD methods declared with attributes, on async and sync connections.

use std::sync::Arc;

use rupa::DynError;

use super::*;

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
    use pollster::block_on as block;

    let a = block(repo.create(&NewNote { title: "a".into() })).expect("create runs");
    let more =
        block(repo.create_many(&[NewNote { title: "b".into() }, NewNote { title: "c".into() }]))
            .expect("create_many runs");
    let imported = repo
        .import(&Note {
            id: 0,
            title: "d".into(),
            body: Some("x".into()),
        })
        .expect("import runs");

    assert_eq!(
        a,
        Note {
            id: 1,
            title: "a".into(),
            body: None
        }
    );
    assert_eq!(more.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(imported, 1);

    let patched = block(repo.patch(&NotePatch {
        id: 1,
        body: Some(Some("hello".into())),
    }))
    .expect("patch runs");
    let mut a = block(repo.get(1))
        .expect("get runs")
        .expect("note 1 exists");

    assert!(patched);
    assert_eq!(a.body.as_deref(), Some("hello"));

    a.title = "a2".into();

    assert_eq!(repo.save(&a).expect("save runs"), 1);
    assert_eq!(
        block(repo.get(1))
            .expect("get runs")
            .expect("note 1 exists")
            .title,
        "a2"
    );
    assert!(block(repo.remove(&a)).expect("remove runs"));
    assert_eq!(block(repo.remove_id(2)).expect("remove_id runs"), 1);
    assert_eq!(block(repo.remove_id(2)).expect("remove_id runs"), 0);
    assert_eq!(
        repo.purge_empty().expect("purge runs"),
        1,
        "only `c` has no body"
    );
    assert_eq!(
        block(repo.get(4)).expect("get runs").map(|n| n.title),
        Some("d".to_string())
    );
}

#[test]
fn crud_repository_over_an_async_connection() {
    let repo: Arc<dyn NoteRepository> = Arc::new(NoteRepositoryRepo::new(note_db().into_async()));

    exercise_notes(&*repo);
}

/// A sync-only application (think SQLite) declaring `async` methods: the
/// sync connection serves them without an adapter or an async runtime.
#[test]
fn crud_repository_over_a_sync_connection() {
    let repo: Arc<dyn NoteRepository> = Arc::new(NoteRepositoryRepo::new(note_db()));

    exercise_notes(&*repo);
}
