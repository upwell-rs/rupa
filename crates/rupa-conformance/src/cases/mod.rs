//! The query, write and DSL cases every driver must pass.

mod dsl;
mod select;
mod write;

use super::*;

pub use dsl::*;
pub use select::*;
pub use write::*;

pub(super) async fn seed<E: AsyncExecutor>(e: &mut E) {
    let all = items();
    let n = e
        .run(insert::<Item>().values(&all).affected())
        .await
        .expect("seed");
    assert_eq!(n, all.len() as u64);
}

pub(super) async fn ids<E: AsyncExecutor>(e: &mut E, cond: Expr<bool>) -> Vec<i64> {
    let rows = e
        .run(
            select::<Item>()
                .filter(cond)
                .order_by(col!(Item::id).asc())
                .all(),
        )
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
pub(super) async fn insert_notes<E: AsyncExecutor>(e: &mut E, notes: &[NewNote]) -> Vec<Note> {
    if returning(e) {
        return e
            .run(insert::<Note>().values(notes).returning_all())
            .await
            .unwrap();
    }

    let n = e
        .run(insert::<Note>().values(notes).affected())
        .await
        .unwrap() as usize;
    let mut newest = e
        .run(
            select::<Note>()
                .order_by(col!(Note::id).desc())
                .limit(n as u64)
                .all(),
        )
        .await
        .unwrap();
    newest.reverse();
    newest
}
