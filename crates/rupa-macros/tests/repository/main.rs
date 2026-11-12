//! `#[repository]` end to end on the memory backend.

mod connections;
mod crud;
mod dispatch;
mod joins;

use rupa::DynError;
use rupa::prelude::*;
use rupa_driver_memory::{AsyncMemoryDb, MemoryDb};

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "users")]
pub struct User {
    #[id]
    id: i64,
    email: String,
    active: bool,
}

/// An application error that repository errors convert into.
#[derive(Debug)]
pub enum AppError {
    Db(#[allow(dead_code)] DynError),
}

impl From<DynError> for AppError {
    fn from(e: DynError) -> Self {
        AppError::Db(e)
    }
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

/// A sync database holding [`users`].
fn user_db() -> MemoryDb {
    let mut db = MemoryDb::new();

    db.register::<User>();
    insert::<User>()
        .values(&users())
        .affected()
        .execute(&db)
        .expect("users inserted");

    db
}

/// [`user_db`] as an async executor.
fn async_user_db() -> AsyncMemoryDb {
    user_db().into_async()
}
