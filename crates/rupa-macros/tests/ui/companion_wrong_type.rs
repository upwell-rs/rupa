use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct User { #[id(generated)] id: i64, email: String }
#[derive(Insertable)]
#[insertable(entity = User)]
struct NewUser { email: Option<String> }
fn main() {}
