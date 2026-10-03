use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct T { #[id] a: i64, #[id] b: i64 }
fn main() {}
