rupa::table!(Accounts = "accounts" {
    #[id]
    id: i64,
});

fn main() {
    let _ = rupa::delete::<Accounts>().one(&Accounts);
}
