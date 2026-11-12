#[path = "../fixtures/mod.rs"]
mod fixtures;
use fixtures::User;
use rupa_core::Expr;
use rupa_core::ExprOps;
use rupa_core::schema::{Gin, IndexKind};

fn main() {
    let lowered = Expr::<i64>::call("abs", [Expr::<i64>::from_node(User::ID.into_expr_node())]);

    let _ = Gin::on_expr::<User>("users_abs_id", lowered);
}
