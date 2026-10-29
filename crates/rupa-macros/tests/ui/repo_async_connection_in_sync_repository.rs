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
    fn all(&self) -> Result<Vec<User>, rupa::DynError>;
}

fn main() {
    let db = rupa_driver_memory::MemoryDb::new().into_async();
    let users = UsersRepo::new(&db);
    let _ = users.all();
}
