use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, email: String }

#[query(filter = User::email == $email)]
fn f(email: &str) -> Vec<User>;
fn main() {}
