//! The query, write and DSL cases every driver must pass.

mod dsl;
mod select;
mod write;

use super::*;

pub use dsl::*;
pub use select::*;
pub use write::*;

pub(super) async fn seed<E: AsyncExecutor>(e: &E) {
    let all = items();
    let n = insert::<Item>()
        .values(&all)
        .affected()
        .execute(e)
        .await
        .expect("seed");
    assert_eq!(n, all.len() as u64);
}

pub(super) async fn ids<E: AsyncExecutor>(e: &E, cond: Expr<bool>) -> Vec<i64> {
    let rows = select::<Item>()
        .filter(cond)
        .order_by(Item::ID.asc())
        .all()
        .execute(e)
        .await
        .expect("select");
    rows.into_iter().map(|i| i.id).collect()
}

pub(super) fn new_note(title: &str) -> NewNote {
    NewNote {
        title: title.into(),
        pinned: false,
    }
}

/// Whether the dialect has `RETURNING`; without it, generated values are
/// read back with a query instead.
pub(super) fn returning<E: AsyncExecutor>(e: &E) -> bool {
    e.dialect().supports(Capability::Returning)
}

/// Inserts notes and returns them as stored (with generated ids).
pub(super) async fn insert_notes<E: AsyncExecutor>(e: &E, notes: &[NewNote]) -> Vec<Note> {
    if returning(e) {
        return insert::<Note>()
            .values(notes)
            .returning_all()
            .execute(e)
            .await
            .unwrap();
    }

    let n = insert::<Note>()
        .values(notes)
        .affected()
        .execute(e)
        .await
        .unwrap() as usize;
    let mut newest = (select::<Note>()
        .order_by(Note::ID.desc())
        .limit(n as u64)
        .all())
    .execute(e)
    .await
    .unwrap();
    newest.reverse();
    newest
}
