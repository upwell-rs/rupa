use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "users")]
pub struct User { #[id] id: i64, name: String }

#[derive(Debug, Joined)]
pub struct View { #[from(User::name)] name: i64 }

fn main() {}
