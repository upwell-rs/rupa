use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct User { #[id] id: i64, other: i64 }
#[derive(Updatable)]
#[updatable(entity = User)]
struct Patch { #[id] other: i64 }
fn main() {}
