use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "t")]
pub struct User { #[id] id: i64, email: String, prefs: Prefs }
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Prefs { theme: String }

#[query(filter = email == $email)]
fn f(email: &str, extra: i64) -> Vec<User>;
fn main() {}
