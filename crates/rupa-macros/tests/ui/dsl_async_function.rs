use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::dsl::{self, Dialect, DslError, Expr};
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String, prefs: serde_json::Value }

#[dsl::function]
async fn f(a: Expr<String>) -> Expr<bool> { todo!() }
fn main() {}
