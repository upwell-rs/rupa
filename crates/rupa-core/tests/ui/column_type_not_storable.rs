use rupa_core::{Column, Json};

struct Opaque;

fn main() {
    let _: Column<(), Opaque, Json> = Column::json("blob");
}
