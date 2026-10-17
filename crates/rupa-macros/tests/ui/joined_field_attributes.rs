use rupa::prelude::*;

#[derive(Debug, Joined)]
pub struct View { #[from(name)] name: String, total: i64 }

fn main() {}
