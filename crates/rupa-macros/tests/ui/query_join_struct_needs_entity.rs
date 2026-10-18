use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, active: bool }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(references = User)] user_id: i64 }

#[derive(Debug, Joined)] pub struct V { #[from(User::id)] id: i64 }
#[query(inner_join = Order)]
fn f() -> Vec<V>;
fn main() {}
