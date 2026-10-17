use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, active: bool }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(references = User)] user_id: i64 }

#[query(left_join = Order, order_by = id)]
fn f() -> Vec<(User, Option<Order>)>;
fn main() {}
