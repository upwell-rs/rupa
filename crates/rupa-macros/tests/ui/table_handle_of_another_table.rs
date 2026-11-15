use rupa::core::schema::Unique;

rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
});

rupa::table!(Teams = "teams" {
    #[id]
    id: i64,
    name: String,
});

fn main() {
    let _ = Unique::on("accounts_name", Accounts::ID).and(Teams::NAME);
}
