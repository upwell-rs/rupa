use rupa::prelude::*;

#[derive(Entity)]
#[entity(table = "users")]
struct User {
    #[id]
    id: i64,
}

#[repository]
trait Users {
    #[query(filter = id == $id)]
    fn by_id(&mut self, id: i64) -> Result<Option<User>, rupa::DynError>;
}

fn main() {}
