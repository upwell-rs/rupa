rupa::table!(Memberships = "memberships" as MembershipRow {
    #[id]
    team_id: i64,
    #[id]
    user_id: i64,
});

fn main() {}
