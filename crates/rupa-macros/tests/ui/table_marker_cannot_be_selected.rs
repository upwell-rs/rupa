rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
});

fn main() {
    let _ = rupa::select::<Accounts>();
}
