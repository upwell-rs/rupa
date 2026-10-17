use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, manager_id: Option<i64> }

fn main() {
    // `User` is a source twice: `on!` can't tell which one is meant.
    let _ = select::<User>().inner_join_on::<User>(on!(User::id == User::manager_id));
}
