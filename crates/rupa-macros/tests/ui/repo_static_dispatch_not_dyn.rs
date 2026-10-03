use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String }

#[repository(static_dispatch)]
pub trait R {
    #[query(filter = id == $id)]
    async fn a(&self, id: i64) -> Result<Option<User>, DynError>;
}
fn take(_: std::sync::Arc<dyn R>) {}
fn main() {}
