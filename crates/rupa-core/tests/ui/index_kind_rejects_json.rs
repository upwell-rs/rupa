#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::schema::{BTree, IndexKind};

fn main() {
    let _ = BTree::on("users_prefs", User::PREFS);
}
