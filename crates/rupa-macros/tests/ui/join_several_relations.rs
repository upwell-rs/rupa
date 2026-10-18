use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64 }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order {
    #[id] id: i64,
    #[column(references = User)] user_id: i64,
    #[column(references = User, relation = reviewer)] reviewed_by: Option<i64>,
}

fn main() {
    // Two relations fit: pick one with `inner_via(Order::reviewer())`.
    let _ = select::<User>().join::<Order>().inner();
}
