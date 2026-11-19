rupa::table!(Memberships = "memberships" {
    #[id]
    team_id: i64,
    #[id]
    user_id: i64,
});

rupa::table!(Invites = "invites" {
    #[id]
    id: i64,
    #[column(references = Memberships)]
    membership_id: i64,
});

fn main() {}
