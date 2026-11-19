use rupa::core::schema::{Gin, IndexKind};

rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
});

fn main() {
    let _ = Gin::on("accounts_id", Accounts::ID);
}
