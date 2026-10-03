use rupa::prelude::*;
#[derive(Entity)]
#[entity(table = "audit")]
struct Audit { #[id] id: i64, message: String }
fn main() {
    let a = Audit { id: 1, message: String::new() };
    let _ = insert::<Audit>().value(&a);
    let _ = delete::<Audit>();
    let _ = get::<Audit>(&1);
}
