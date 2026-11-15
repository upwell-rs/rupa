rupa::table!(Owners = "owners" {
    #[id]
    id: i64,
});

rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
    #[column(references = Owners)]
    owner_id: String,
});

fn main() {}
