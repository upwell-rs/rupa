rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
    #[unique]
    email: String,
});

fn main() {}
