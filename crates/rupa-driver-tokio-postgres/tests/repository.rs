//! A dyn repository over a real Postgres, as DI would hold it: `Arc<dyn Trait>`.

use std::sync::Arc;

use rupa::DynError;
use rupa::prelude::*;
use rupa_conformance::{NewNote, Note, NotePatch, POSTGRES_DDL};
use rupa_driver_tokio_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[repository]
pub trait Notes: Send + Sync {
    #[query(filter = pinned == $pinned, order_by = id)]
    async fn pinned(&self, pinned: bool) -> Result<Vec<Note>, DynError>;

    #[query(filter = title like $pattern)]
    fn matching(&self, pattern: &str) -> Result<Vec<Note>, DynError>;

    #[query(insert, returning)]
    async fn create(&self, note: &NewNote) -> Result<Note, DynError>;

    #[query(get)]
    async fn get(&self, id: i64) -> Result<Option<Note>, DynError>;

    #[query(update)]
    async fn patch(&self, patch: &NotePatch) -> Result<bool, DynError>;

    #[query(delete, entity = Note)]
    async fn remove(&self, id: i64) -> Result<bool, DynError>;
}

#[tokio::test(flavor = "multi_thread")]
async fn dyn_repository_on_postgres() {
    let node = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    client.batch_execute(POSTGRES_DDL).await.unwrap();

    let mut exec = PgExecutor::new(client);
    {
        use rupa::core::exec::AsyncExecutor;
        let notes = [("alpha", true), ("beta", false), ("alphabet", true)].map(|(t, p)| NewNote {
            title: t.into(),
            pinned: p,
        });
        exec.run(insert::<Note>().values(&notes).affected())
            .await
            .unwrap();
    }

    let repo: Arc<dyn Notes> = Arc::new(Repo::shared_async(exec));
    let pinned = repo.pinned(true).await.unwrap();
    assert_eq!(
        pinned.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(),
        ["alpha", "alphabet"]
    );

    // The repository is shareable across tasks...
    let r2 = repo.clone();
    let spawned = tokio::spawn(async move { r2.pinned(false).await.unwrap().len() });
    assert_eq!(spawned.await.unwrap(), 1);

    // ...and its sync method blocks in place without stalling the runtime.
    let matching = repo.matching("alpha%").unwrap();
    assert_eq!(matching.len(), 2);

    // CRUD methods, with the database generating the id.
    let created = repo
        .create(&NewNote {
            title: "gamma".into(),
            pinned: false,
        })
        .await
        .unwrap();
    assert_eq!(created.id, 4);
    let patch = NotePatch {
        id: created.id,
        body: Some(Some("text".into())),
        pinned: None,
    };
    assert!(repo.patch(&patch).await.unwrap());
    assert_eq!(
        repo.get(created.id).await.unwrap().unwrap().body.as_deref(),
        Some("text")
    );
    assert!(repo.remove(created.id).await.unwrap());
    assert!(repo.get(created.id).await.unwrap().is_none());
}
