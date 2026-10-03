use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::dsl::{self, Dialect, DslError, Expr};
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String, prefs: serde_json::Value }

#[dsl::function(requires = Telepathy)]
fn f(a: Expr<String>) -> Expr<bool> { Expr::raw_op("=", a.clone(), a) }
fn main() {}
