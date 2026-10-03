#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;

fn main() {
    let _ = User::EMAIL.path("x");
}
