use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, manager_id: Option<i64> }

#[query(left_join = User on User::id == User::manager_id)]
fn f() -> Vec<(User, Option<User>)>;
fn main() {}
