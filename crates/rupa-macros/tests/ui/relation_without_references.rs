use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "orders")]
pub struct Order { #[id] id: i64, #[column(relation = user)] user_id: i64 }

fn main() {}
