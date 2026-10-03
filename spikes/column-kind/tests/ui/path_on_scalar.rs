use rupa_spike_column_kind::{Entity, col};

#[derive(Entity)]
#[entity(table = "t")]
struct T {
    #[id]
    id: i64,
    email: String,
}

fn main() {
    let _ = col!(T::email).path("x");
}
