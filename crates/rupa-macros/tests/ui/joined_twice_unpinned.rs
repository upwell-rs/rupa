use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, manager_id: Option<i64> }

/// `User` is a source twice: the plain field must say which one.
#[derive(Debug, Joined)]
pub struct Pair { #[from(User::id)] id: i64 }

fn main() {
    let _ = select::<User>()
        .left_join_on::<User>(|(e, m)| m.col(User::ID).eq(e.col(User::MANAGER_ID)))
        .into::<Pair>()
        .all();
}
