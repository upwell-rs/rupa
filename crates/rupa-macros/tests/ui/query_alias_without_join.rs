use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, manager_id: Option<i64> }

#[query(entity = User as e, filter = manager_id is_null)]
fn f() -> Vec<User>;
fn main() {}
