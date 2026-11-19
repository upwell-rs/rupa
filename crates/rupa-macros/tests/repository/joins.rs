//! Joins in a repository.

use std::sync::Arc;

use rupa::DynError;

use super::*;

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "posts")]
pub struct Post {
    #[id]
    id: i64,
    #[column(references = User)]
    user_id: i64,
    title: String,
}

#[derive(Debug, PartialEq, Joined)]
pub struct Byline {
    #[from(Post::title)]
    title: String,
    #[from(User::email)]
    author: String,
}

#[repository]
pub trait PostRepository: Send + Sync {
    #[query(left_join = Post, filter = User::active == $active, order_by = User::id, order_by = Post::id)]
    async fn with_posts(&self, active: bool) -> Result<Vec<(User, Option<Post>)>, DynError>;

    #[query(entity = Post, inner_join = User on User::id == Post::user_id, order_by = Post::id desc)]
    fn bylines(&self) -> Result<Vec<Byline>, DynError>;
}

#[test]
fn joins_in_a_repository() {
    let mut db = user_db();
    let posts =
        [(10, 1, "hello"), (11, 1, "again"), (12, 2, "hi")].map(|(id, user_id, title)| Post {
            id,
            user_id,
            title: title.into(),
        });

    db.register::<Post>();
    insert::<Post>()
        .values(&posts)
        .affected()
        .execute(&db)
        .expect("posts inserted");

    let repo: Arc<dyn PostRepository> = Arc::new(PostRepositoryRepo::new(db.into_async()));
    let rows = pollster::block_on(repo.with_posts(true)).expect("with_posts runs");
    let ids: Vec<_> = rows
        .iter()
        .map(|(u, p)| (u.id, p.as_ref().map(|p| p.id)))
        .collect();
    let bylines = repo.bylines().expect("bylines runs");

    assert_eq!(ids, [(1, Some(10)), (1, Some(11)), (3, None)]);
    assert_eq!(
        bylines,
        [("hi", "bob@x"), ("again", "ada@x"), ("hello", "ada@x")].map(|(title, author)| Byline {
            title: title.into(),
            author: author.into(),
        })
    );
}
