use rupa::prelude::*;
#[derive(Debug, Entity)]
#[entity(table = "types")]
pub struct Type { #[id] id: i64 }
#[derive(Debug, Entity)]
#[entity(table = "things")]
pub struct Thing { #[id] id: i64, #[column(references = Type)] type_id: i64 }

fn main() {}
