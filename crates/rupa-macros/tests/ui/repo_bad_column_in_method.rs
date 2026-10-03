use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String }

#[repository]
pub trait R {
    #[query(filter = mail == $m)]
    fn a(&self, m: &str) -> Result<Vec<User>, DynError>;
}
fn main() {}
