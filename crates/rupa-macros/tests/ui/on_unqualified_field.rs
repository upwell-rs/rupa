use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64 }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, user_id: i64 }

fn main() {
    let _ = select::<User>().inner_join_on::<Order>(on!(user_id == User::id));
}
