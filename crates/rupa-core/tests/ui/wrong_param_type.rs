#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::prelude::*;

fn main() {
    let _ = select::<User>().filter(User::EMAIL.eq(5i64));
}
