use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, manager_id: Option<i64> }

#[query(entity = User as e, left_join = User as m on User::id == m::manager_id)]
fn f() -> Vec<(User, Option<User>)>;
fn main() {}
