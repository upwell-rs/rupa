rupa::table!(Owners = "owners" {
    #[id]
    id: i64,
});

rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
    #[column(references = Owners, relation = owner)]
    owner_id: i64,
});

fn main() {}
