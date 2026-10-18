use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, active: bool }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(references = User)] user_id: i64 }

#[query(sql = "SELECT 1", inner_join = Order)]
fn f() -> Vec<(User, Order)>;
fn main() {}
