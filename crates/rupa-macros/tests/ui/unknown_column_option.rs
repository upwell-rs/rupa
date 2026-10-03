use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct T { #[id] id: i64, #[column(nmae = "x")] name: String }
fn main() {}
