use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "t")]
struct User { #[id(generated)] id: i64, email: String, nickname: Option<String> }
#[derive(Insertable)]
#[insertable(entity = User)]
struct NewUser { nickname: Option<String> }
fn main() {}
