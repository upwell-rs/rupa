use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct T<V> { #[id] id: i64, v: V }
fn main() {}
