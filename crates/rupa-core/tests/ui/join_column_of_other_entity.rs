#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::prelude::*;

struct Other;

fn main() {
    let foreign: Column<Other, i64, Scalar> = Column::scalar("id");
    // A source handle only takes its own entity's columns.
    let _ = select::<User>().inner_join_on::<User>(|(a, b)| b.col(foreign).eq(a.col(User::ID)));
}
