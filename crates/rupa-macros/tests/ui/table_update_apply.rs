rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
});

fn main() {
    let _ = rupa::update::<Accounts>().apply(&Accounts);
}
