use rupa::prelude::*;

#[derive(Entity)]
#[entity(table = "users")]
struct User {
    #[id]
    id: i64,
}

#[repository]
trait Users {
    #[query(order_by = id)]
    async fn all(&self) -> Result<Vec<User>, rupa::DynError>;
}

fn main() {
    let source = rupa::Shared::new(rupa_driver_memory::MemoryDb::new());
    let users = UsersView::new(&source);
    let _ = users.all();
}
