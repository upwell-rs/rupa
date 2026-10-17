use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, name: String }
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(references = User)] user_id: i64 }
#[derive(Debug, Entity)]
#[entity(table = "coupons")]
pub struct Coupon { #[id] id: i64, code: String }

#[derive(Debug, Joined)]
pub struct View { #[from(User::name)] name: String, #[from(Coupon::code)] code: String }

fn main() {
    let _ = select::<User>().join::<Order>().inner().into::<View>().all();
}
