use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity, Gettable, Insertable)]
#[entity(table = "t")]
pub struct Note { #[id(generated)] id: i64, title: String }
pub struct NotInsertable { pub title: String }

#[repository]
pub trait R {
    #[query(update)]
    fn save(&self, n: &Note) -> Result<u64, DynError>;
}
fn main() {}
