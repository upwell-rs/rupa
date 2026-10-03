use rupa_core::dialect::{DialectId, DynDialect, Supports, caps};

fn needs_ilike<D: Supports<caps::Ilike>>(_: &D) {}

fn main() {
    needs_ilike(&DynDialect::new(DialectId::Postgres));
}
