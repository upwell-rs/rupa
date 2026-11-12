use rupa::Entity;

#[derive(Entity)]
#[entity(table = "t")]
struct T { #[id] id: i64, __rupa_insert_required: i64 }

fn main() {}
