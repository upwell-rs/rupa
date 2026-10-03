use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String }

#[derive(Debug)]
pub struct MyError;
#[repository]
pub trait R {
    #[query(filter = id == $id)]
    fn a(&self, id: i64) -> Result<Option<User>, MyError>;
}
fn check(db: rupa_driver_memory::MemoryDb) {
    let repo = Repo::shared(db);
    let _ = R::a(&repo, 1);
}
fn main() {}
