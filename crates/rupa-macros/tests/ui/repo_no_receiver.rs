use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String }

#[repository]
pub trait R {
    #[query(filter = id == $id)]
    fn a(id: i64) -> Result<Option<User>, DynError>;
}
fn main() {}
