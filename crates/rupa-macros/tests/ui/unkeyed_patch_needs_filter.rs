use rupa::prelude::*;
#[derive(Entity, Updatable)]
#[entity(table = "t")]
struct User { #[id] id: i64, email: String }
#[derive(Updatable)]
#[updatable(entity = User)]
struct Rename { email: Option<String> }
fn main() {
    let _ = update::<User>().one(&Rename { email: None });
}
