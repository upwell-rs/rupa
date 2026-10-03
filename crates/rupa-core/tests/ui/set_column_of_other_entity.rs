#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::prelude::*;

struct Other;

fn main() {
    let foreign: Column<Other, String, Scalar> = Column::scalar("email");
    let _ = update::<User>().set(foreign, &"x".to_string());
}
