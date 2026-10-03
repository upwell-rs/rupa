use rupa::prelude::*;
#[allow(unused_imports)]
use rupa::dsl::{self, Dialect, DslError, Expr};
#[allow(unused_imports)]
use rupa::DynError;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String, prefs: serde_json::Value }

#[query(filter = no_such_fn(email, $p))]
fn f(p: &str) -> Vec<User>;
fn main() {}
