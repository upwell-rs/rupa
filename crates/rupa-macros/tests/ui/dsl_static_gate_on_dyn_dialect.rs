use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::dsl::{self, Dialect, DslError, Expr};
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String, prefs: serde_json::Value }

#[repository]
pub trait Users: Send + Sync {
    #[query(filter = dsl::json_contains(prefs, $part))]
    async fn with_prefs(&self, part: serde_json::Value) -> Result<Vec<User>, DynError>;
}
fn wire(exec: rupa::core::exec::BoxAsyncExecutor) -> std::sync::Arc<dyn Users> {
    std::sync::Arc::new(Repo::shared_async(exec))
}
fn main() {}
