use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, name: String }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(references = User)] user_id: i64, total: i64 }

#[derive(Debug, Joined)]
pub struct View { #[from(User::name)] name: String, #[from(Order::total)] total: i64 }

fn main() {
    // Order is left-joined: `total` must be an Option.
    let _ = select::<User>().join::<Order>().left().into::<View>().all();
}
