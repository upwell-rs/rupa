#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::prelude::*;

fn main() {
    let _ = insert::<User>().value(&User::sample()).build::<Vec<User>>();
}
