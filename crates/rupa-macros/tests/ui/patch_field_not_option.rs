use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct User { #[id] id: i64, email: String }
#[derive(Updatable)]
#[updatable(entity = User)]
struct Patch { email: String }
fn main() {}
