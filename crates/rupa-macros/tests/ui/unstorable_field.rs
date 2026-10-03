use rupa::prelude::*;
struct Opaque;
#[derive(Entity)]
#[entity(table = "t")]
struct T { #[id] id: i64, blob: Opaque }
fn main() {}
