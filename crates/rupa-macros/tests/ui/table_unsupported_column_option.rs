rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
    #[column(name = "mail")]
    email: String,
});

fn main() {}
