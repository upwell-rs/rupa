#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::schema::{Gist, IndexKind};

fn main() {
    let _ = Gist::on("users_prefs", User::PREFS);
}
