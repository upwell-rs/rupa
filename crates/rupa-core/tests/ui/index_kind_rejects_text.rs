#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::schema::{Gin, IndexKind};

fn main() {
    let _ = Gin::on("users_email", User::EMAIL);
}
